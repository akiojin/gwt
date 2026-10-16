//! Caller-owned attempt control, published before initial host admission.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gwt_core::index_coordinator::{IndexCoordinator, TargetKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{interruption, VerificationCallerAuthority};
use crate::cli::trusted_store;

const ADMISSION_INTERRUPTED: &str = "external termination during admission wait: runner pipe closed before admission; rerun required";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AttemptStatus {
    Waiting,
    Running,
    Deferred,
    Interrupted,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AttemptRecord {
    pub attempt_id: String,
    pub session_id: String,
    authority_hash: String,
    repo_hash: String,
    worktree_hash: String,
    pub commands: Vec<String>,
    pub status: AttemptStatus,
    pub reason: Option<String>,
    pub record_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    external_termination: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_terminations: Option<u8>,
    created_at: chrono::DateTime<chrono::Utc>,
    watchdog_token_hash: String,
    runner_pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runner_started_at: Option<u64>,
    content_hash: String,
}

impl AttemptRecord {
    fn key(&self) -> TargetKey {
        TargetKey::verification(&self.repo_hash, &self.worktree_hash)
    }
}

pub(super) struct Attempt {
    worktree: PathBuf,
    id: String,
}

impl Attempt {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn cancelled(&self) -> io::Result<bool> {
        Ok(matches!(
            required(&self.worktree, &self.id)?.status,
            AttemptStatus::Interrupted | AttemptStatus::Completed
        ))
    }
    pub fn ensure_active(&self) -> io::Result<()> {
        if self.cancelled()? {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "verification attempt canceled",
            ))
        } else {
            Ok(())
        }
    }
    pub fn mark_running(&self, record_id: &str) -> io::Result<()> {
        trusted_store::with_write_lease(&self.worktree, || {
            self.ensure_active()?;
            let mut record = required(&self.worktree, &self.id)?;
            record.status = AttemptStatus::Running;
            record.record_id = Some(record_id.to_string());
            save(&self.worktree, &mut record)
        })
    }
    pub fn set_admission_waiting(&self, waiting: bool) -> io::Result<()> {
        trusted_store::with_write_lease(&self.worktree, || {
            self.ensure_active()?;
            let mut record = required(&self.worktree, &self.id)?;
            let status = if waiting || record.record_id.is_none() {
                AttemptStatus::Waiting
            } else {
                AttemptStatus::Running
            };
            if record.status != status {
                record.status = status;
                save(&self.worktree, &mut record)?;
            }
            Ok(())
        })
    }
    /// Caller already owns the trusted write lease, shared with the VRR commit.
    pub fn complete_under_lease(&self) -> io::Result<()> {
        self.ensure_active()?;
        let mut record = required(&self.worktree, &self.id)?;
        record.status = AttemptStatus::Completed;
        save(&self.worktree, &mut record)
    }
    /// Preserve retry history before the final VRR drops its lifecycle.
    pub fn retain_termination_budget_under_lease(&self, count: Option<u8>) -> io::Result<()> {
        self.ensure_active()?;
        let mut record = required(&self.worktree, &self.id)?;
        record.external_terminations = count;
        save(&self.worktree, &mut record)
    }
    pub fn returned(&self, error: Option<&str>, coordinator: &IndexCoordinator) -> io::Result<()> {
        trusted_store::with_write_lease(&self.worktree, || {
            let mut record = required(&self.worktree, &self.id)?;
            if record.status == AttemptStatus::Interrupted {
                return settle_interrupted_under_lease(&self.worktree, &record, coordinator);
            }
            if record.status == AttemptStatus::Completed {
                return Ok(());
            }
            if error.is_some_and(|error| error.contains("verify: deferred")) {
                record.status = AttemptStatus::Deferred;
                record.reason = error.map(str::to_owned);
                save(&self.worktree, &mut record)
            } else {
                interrupt(
                    &self.worktree,
                    &mut record,
                    error.unwrap_or("verification runner returned without a terminal result"),
                    false,
                    coordinator,
                )
            }
        })
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn authority_hash(authority: &VerificationCallerAuthority) -> io::Result<String> {
    Ok(digest(&serde_json::to_vec(authority)?))
}
fn filename(id: &str) -> io::Result<String> {
    if !id
        .strip_prefix("vat-")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(not_yours());
    }
    Ok(format!("verification-attempt-{id}.json"))
}
fn latest_filename(session: &str) -> String {
    format!(
        "verification-attempt-latest-{}.json",
        digest(session.as_bytes())
    )
}
fn read(worktree: &Path, name: &str) -> io::Result<Option<String>> {
    if let Some(contents) = trusted_store::read(worktree, name)? {
        return Ok(Some(contents));
    }
    // Attempt control has no legacy mirror import: a git worktree requires
    // its canonical trusted copy. Non-git unit fixtures use the mirror.
    if trusted_store::trusted_dir_for_worktree(worktree).is_some() {
        return Ok(None);
    }
    match fs::read_to_string(worktree.join(".gwt/skill-state").join(name)) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
pub(super) fn load(worktree: &Path, id: &str) -> io::Result<Option<AttemptRecord>> {
    let Some(contents) = read(worktree, &filename(id)?)? else {
        return Ok(None);
    };
    let record: AttemptRecord = serde_json::from_str(&contents)?;
    let mut unsigned = record.clone();
    unsigned.content_hash.clear();
    if record.attempt_id != id || record.content_hash != digest(&serde_json::to_vec(&unsigned)?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "verification attempt integrity mismatch",
        ));
    }
    Ok(Some(record))
}
fn required(worktree: &Path, id: &str) -> io::Result<AttemptRecord> {
    load(worktree, id)?.ok_or_else(not_yours)
}
fn save(worktree: &Path, record: &mut AttemptRecord) -> io::Result<()> {
    record.content_hash.clear();
    record.content_hash = digest(&serde_json::to_vec(record)?);
    let name = filename(&record.attempt_id)?;
    trusted_store::write_with_mirror(
        worktree,
        &name,
        &worktree.join(".gwt/skill-state").join(&name),
        &serde_json::to_vec_pretty(record)?,
    )
}
fn not_yours() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "not your verification attempt",
    )
}

pub(super) fn admission_resume_candidate(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
) -> io::Result<Option<Vec<String>>> {
    resume_candidate(worktree, id, token, session, false)
}

pub(super) fn admitted_resume_candidate(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
) -> io::Result<Option<Vec<String>>> {
    resume_candidate(worktree, id, token, session, true)
}

fn resume_candidate(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
    admitted: bool,
) -> io::Result<Option<Vec<String>>> {
    let record = required(worktree, id)?;
    if record.watchdog_token_hash != digest(token.as_bytes()) || record.session_id != session {
        return Err(not_yours());
    }
    let eligible = if admitted {
        // The tool may finish before the Agent's busy pane becomes idle.
        matches!(
            record.status,
            AttemptStatus::Waiting | AttemptStatus::Running | AttemptStatus::Completed
        )
    } else {
        record.status == AttemptStatus::Deferred
            || (record.status == AttemptStatus::Interrupted
                && record.external_termination
                && record.reason.as_deref() == Some(ADMISSION_INTERRUPTED))
    };
    if read(worktree, &latest_filename(session))?.as_deref() != Some(id) || !eligible {
        return Ok(None);
    }
    // A companion may outlive its producing generation. It must never wake
    // a successor, a settled execution, or a caller whose authority rotated.
    let authority = super::snapshot_verification_caller_authority(worktree, session)?;
    if record.authority_hash != authority_hash(&authority)? {
        return Ok(None);
    }
    Ok(Some(record.commands))
}

pub(super) fn begin_for_caller(
    worktree: &Path,
    session: &str,
    authority: &VerificationCallerAuthority,
    key: &TargetKey,
    commands: &[String],
    token: &str,
) -> io::Result<Attempt> {
    trusted_store::with_write_lease(worktree, || {
        super::revalidate_verification_caller_authority(worktree, session, authority)?;
        let mut record = AttemptRecord {
            attempt_id: format!("vat-{}", uuid::Uuid::new_v4().simple()),
            session_id: session.to_string(),
            authority_hash: authority_hash(authority)?,
            repo_hash: key.repo_hash().to_string(),
            worktree_hash: key.worktree_hash().unwrap_or_default().to_string(),
            commands: commands.to_vec(),
            status: AttemptStatus::Waiting,
            reason: None,
            record_id: None,
            external_termination: false,
            external_terminations: None,
            created_at: chrono::Utc::now(),
            watchdog_token_hash: digest(token.as_bytes()),
            runner_pid: std::process::id(),
            runner_started_at: crate::process::host_process_start_time(std::process::id()),
            content_hash: String::new(),
        };
        save(worktree, &mut record)?;
        let name = latest_filename(session);
        trusted_store::write_with_mirror(
            worktree,
            &name,
            &worktree.join(".gwt/skill-state").join(&name),
            record.attempt_id.as_bytes(),
        )?;
        Ok(Attempt {
            worktree: worktree.to_path_buf(),
            id: record.attempt_id,
        })
    })
}

fn clear(coordinator: &IndexCoordinator, record: &AttemptRecord) -> io::Result<()> {
    coordinator
        .clear_heavy_reservation_for_attempt(&record.key(), &record.attempt_id)
        .map(|_| ())
        .map_err(io::Error::other)
}
fn interrupt(
    worktree: &Path,
    record: &mut AttemptRecord,
    reason: &str,
    external: bool,
    coordinator: &IndexCoordinator,
) -> io::Result<()> {
    record.status = AttemptStatus::Interrupted;
    record.reason = Some(reason.to_string());
    record.external_termination = external;
    // Persist the cancellation fence before touching queue metadata. Admission
    // checks this copy under its queue lock and cannot resurrect the attempt.
    save(worktree, record)?;
    settle_interrupted_under_lease(worktree, record, coordinator)
}
fn settle_interrupted_under_lease(
    worktree: &Path,
    record: &AttemptRecord,
    coordinator: &IndexCoordinator,
) -> io::Result<()> {
    clear(coordinator, record)?;
    if let Some(id) = &record.record_id {
        interruption::interrupt_matching_record(
            worktree,
            id,
            &record.attempt_id,
            record
                .reason
                .as_deref()
                .unwrap_or("verification interrupted; rerun required"),
            record.external_termination,
            interruption::RunLifecycle {
                status: interruption::RunStatus::Running,
                runner_pid: record.runner_pid,
                runner_started_at: record.runner_started_at,
                watchdog_token_hash: record.watchdog_token_hash.clone(),
                current_command: None,
                reason: None,
                external_terminations: record.external_terminations,
            },
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn cancel_for_caller(
    worktree: &Path,
    session: &str,
    id: &str,
    reason: &str,
    authority: &VerificationCallerAuthority,
    coordinator: &IndexCoordinator,
    key: &TargetKey,
) -> io::Result<AttemptRecord> {
    if reason.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cancellation reason is required",
        ));
    }
    trusted_store::with_write_lease(worktree, || {
        let mut record = required(worktree, id)?;
        if record.session_id != session
            || record.key() != *key
            || record.authority_hash != authority_hash(authority)?
        {
            return Err(not_yours());
        }
        super::revalidate_verification_caller_authority(worktree, session, authority)?;
        if record.status == AttemptStatus::Completed {
            return Err(io::Error::other("verification attempt already completed"));
        }
        if record.status != AttemptStatus::Interrupted {
            interrupt(
                worktree,
                &mut record,
                &format!("caller cancellation: {}", reason.trim()),
                false,
                coordinator,
            )?;
        } else {
            if record.reason.as_deref() == Some(ADMISSION_INTERRUPTED) {
                // Cancel the pending self-notification while preserving the
                // authenticated external-termination history and retry budget.
                record.reason = Some(format!("caller cancellation: {}", reason.trim()));
                save(worktree, &mut record)?;
            }
            settle_interrupted_under_lease(worktree, &record, coordinator)?;
        }
        Ok(record)
    })
}

pub(super) fn status(
    worktree: &Path,
    session: &str,
    id: Option<&str>,
    coordinator: &IndexCoordinator,
) -> io::Result<serde_json::Value> {
    let selected = id
        .map(str::to_owned)
        .map(Ok)
        .unwrap_or_else(|| read(worktree, &latest_filename(session))?.ok_or_else(not_yours))?;
    let record = required(worktree, &selected)?;
    let mut value = serde_json::to_value(&record)?;
    // Do not expose private token or authority digests through public status.
    for name in ["authority_hash", "watchdog_token_hash", "content_hash"] {
        value.as_object_mut().expect("record object").remove(name);
    }
    value["reservation"] = coordinator
        .heavy_lease_status()
        .map_err(io::Error::other)?
        .queue
        .iter()
        .any(|entry| {
            entry.target.as_deref() == Some(record.key().file_stem().as_str())
                && entry.attempt_id.as_deref() == Some(record.attempt_id.as_str())
        })
        .into();
    Ok(value)
}

/// The hash-covered VRR binding survives replacement of the latest-attempt
/// pointer. A final VRR/control two-write boundary cannot expose PASS early.
pub(super) fn finalized_status(
    worktree: &Path,
    record: &super::VerificationRunRecord,
) -> io::Result<Option<AttemptStatus>> {
    Ok(bound_attempt(worktree, record)?.map(|attempt| attempt.status))
}
fn bound_attempt(
    worktree: &Path,
    record: &super::VerificationRunRecord,
) -> io::Result<Option<AttemptRecord>> {
    let extensions = record.unknown_fields();
    let Some(id) = extensions.get("verification_attempt_id") else {
        return Ok(None);
    };
    let id = id.as_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid verification attempt binding",
        )
    })?;
    let attempt = required(worktree, id)?;
    if attempt.record_id.as_deref() != Some(record.record_id.as_str())
        || attempt.session_id != record.session_id
    {
        return Err(not_yours());
    }
    Ok(Some(attempt))
}

/// Call under the trusted lease; settle the exact dead attempt and its queue
/// reservation together with its VRR, including the final-write gap.
pub(super) fn recover_unfinished_run(
    worktree: &Path,
    record: &super::VerificationRunRecord,
) -> io::Result<bool> {
    let Some(mut attempt) = bound_attempt(worktree, record)? else {
        return Ok(false);
    };
    if attempt.status == AttemptStatus::Running {
        if interruption::runner_is_alive(
            attempt.runner_pid,
            attempt.runner_started_at,
            attempt.created_at,
        ) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the previous verification runner is unsettled; wait for its terminal result before retrying (its watchdog may still be recording an external termination)",
            ));
        }
        let coordinator =
            crate::cli::verification_lease::open_coordinator().map_err(io::Error::other)?;
        interrupt(
            worktree,
            &mut attempt,
            "external termination: runner is no longer alive and no terminal result was recorded; signal unknown; rerun required",
            true,
            &coordinator,
        )?;
        return Ok(true);
    }
    if attempt.status != AttemptStatus::Interrupted {
        return Ok(false);
    }
    let coordinator =
        crate::cli::verification_lease::open_coordinator().map_err(io::Error::other)?;
    settle_interrupted_under_lease(worktree, &attempt, &coordinator)?;
    Ok(true)
}

pub(super) fn settle_watchdog(
    worktree: &Path,
    id: &str,
    token: &str,
    normal_return: bool,
) -> io::Result<()> {
    let coordinator =
        crate::cli::verification_lease::open_coordinator().map_err(io::Error::other)?;
    settle_watchdog_with_coordinator(worktree, id, token, normal_return, &coordinator)
}
fn settle_watchdog_with_coordinator(
    worktree: &Path,
    id: &str,
    token: &str,
    normal_return: bool,
    coordinator: &IndexCoordinator,
) -> io::Result<()> {
    trusted_store::with_write_lease(worktree, || {
        let mut record = required(worktree, id)?;
        if record.watchdog_token_hash != digest(token.as_bytes()) {
            return Err(not_yours());
        }
        if record.status == AttemptStatus::Interrupted {
            return settle_interrupted_under_lease(worktree, &record, coordinator);
        }
        if matches!(
            record.status,
            AttemptStatus::Completed | AttemptStatus::Deferred
        ) {
            return Ok(());
        }
        let waiting_for_admission = record.status == AttemptStatus::Waiting;
        interrupt(
            worktree,
            &mut record,
            if normal_return {
                "verification runner returned without a terminal result; rerun required"
            } else if waiting_for_admission {
                ADMISSION_INTERRUPTED
            } else {
                "external termination: runner pipe closed before a terminal result; signal unknown; rerun required"
            },
            !normal_return,
            coordinator,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::verification_record::VerificationCallerAuthority;
    use gwt_core::index_coordinator::{IndexCoordinator, JobPriority, TargetKey};
    use std::time::Duration;

    fn authority() -> VerificationCallerAuthority {
        VerificationCallerAuthority {
            owner_number: None,
            execution_binding: None,
            session_binding: None,
        }
    }

    #[test]
    fn admission_resume_tracks_deferred_and_dead_waiters_but_not_canceled_or_superseded() {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open(runtime.path()).unwrap();
        let key = TargetKey::verification("mine", "worktree");
        let commands = vec!["cargo test --workspace".to_string()];
        let attempt =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &commands, "token").unwrap();
        assert!(
            admission_resume_candidate(repo.path(), attempt.id(), "token", "mine")
                .unwrap()
                .is_none(),
            "a live admission must not be duplicated"
        );
        attempt
            .returned(Some("verify: deferred"), &coordinator)
            .unwrap();
        assert_eq!(
            admission_resume_candidate(repo.path(), attempt.id(), "token", "mine").unwrap(),
            Some(commands.clone()),
            "deferred admission must arrange its own resume"
        );
        assert!(admission_resume_candidate(repo.path(), attempt.id(), "wrong", "mine").is_err());
        assert!(admission_resume_candidate(repo.path(), attempt.id(), "token", "foreign").is_err());
        cancel_for_caller(
            repo.path(),
            "mine",
            attempt.id(),
            "stop",
            &authority(),
            &coordinator,
            &key,
        )
        .unwrap();
        assert!(
            admission_resume_candidate(repo.path(), attempt.id(), "token", "mine")
                .unwrap()
                .is_none()
        );

        let waiter =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &commands, "next").unwrap();
        settle_watchdog_with_coordinator(repo.path(), waiter.id(), "next", false, &coordinator)
            .unwrap();
        assert_eq!(
            admission_resume_candidate(repo.path(), waiter.id(), "next", "mine").unwrap(),
            Some(commands.clone()),
            "runner death during admission must also resume"
        );
        let successor =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &commands, "new").unwrap();
        assert!(
            admission_resume_candidate(repo.path(), waiter.id(), "next", "mine")
                .unwrap()
                .is_none(),
            "an old companion must not wake a newer attempt"
        );
        settle_watchdog_with_coordinator(repo.path(), successor.id(), "new", false, &coordinator)
            .unwrap();
        cancel_for_caller(
            repo.path(),
            "mine",
            successor.id(),
            "stop after runner death",
            &authority(),
            &coordinator,
            &key,
        )
        .unwrap();
        assert!(
            admission_resume_candidate(repo.path(), successor.id(), "new", "mine")
                .unwrap()
                .is_none(),
            "explicit cancellation must suppress resume after admission interruption"
        );
    }

    #[test]
    fn admission_resume_three_agents_finish_after_a_full_pool_without_pm_intervention() {
        use gwt_core::index_coordinator::{JobAdmission, JobOutcome};
        let runtime = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open_verification(runtime.path(), 3).unwrap();
        let job = |label: &str| {
            let key = TargetKey::verification("resume-three", label);
            match coordinator
                .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(10))
                .unwrap()
            {
                JobAdmission::Owner(guard) => guard,
                _ => panic!("expected a new target owner"),
            }
        };
        let holders = (0..3)
            .map(|index| job(&format!("holder-{index}")))
            .collect::<Vec<_>>();
        let leases = holders
            .iter()
            .map(|guard| guard.acquire_heavy(Duration::ZERO).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(coordinator.heavy_pool_status().unwrap().available, 0);
        let waiters = (0..3)
            .map(|index| {
                let repo = tempfile::tempdir().unwrap();
                let key = TargetKey::verification("resume-three", format!("waiter-{index}"));
                let commands = vec!["cargo test --workspace".to_string()];
                let attempt =
                    begin_for_caller(repo.path(), "mine", &authority(), &key, &commands, "token")
                        .unwrap();
                attempt
                    .returned(Some("verify: deferred"), &coordinator)
                    .unwrap();
                (repo, attempt, job(&format!("waiter-{index}")))
            })
            .collect::<Vec<_>>();
        for (repo, attempt, _) in &waiters {
            assert!(super::super::admission_resume::poll(
                repo.path(),
                attempt.id(),
                "token",
                "mine",
                &coordinator,
                |_| panic!("no notification while the pool is full")
            )
            .unwrap());
        }
        drop(leases);
        let mut completed = 0;
        for (repo, attempt, guard) in waiters {
            assert!(!super::super::admission_resume::poll(
                repo.path(),
                attempt.id(),
                "token",
                "mine",
                &coordinator,
                |commands| {
                    assert_eq!(commands, ["cargo test --workspace"]);
                    let _lease = guard.acquire_heavy(Duration::from_secs(10)).unwrap();
                    trusted_store::with_write_lease(repo.path(), || attempt.complete_under_lease())
                        .unwrap();
                    completed += 1;
                    Ok(true)
                }
            )
            .unwrap());
            assert!(!super::super::admission_resume::poll(
                repo.path(),
                attempt.id(),
                "token",
                "mine",
                &coordinator,
                |_| panic!("a completed waiter must never be notified again")
            )
            .unwrap());
            guard.complete(JobOutcome::Completed).unwrap();
        }
        assert_eq!(completed, 3);
    }

    #[test]
    fn caller_cancel_releases_only_its_attempt_and_retains_interruption() {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open(runtime.path()).unwrap();
        let key = TargetKey::verification("mine", "worktree");
        let foreign = TargetKey::verification("other-project", "worktree");
        let commands = vec!["cargo test --workspace".to_string()];
        let attempt =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &commands, "token").unwrap();
        let check = || attempt.cancelled().map_err(Into::into);
        coordinator
            .reserve_heavy_for_attempt(
                &key,
                JobPriority::ManualRebuild,
                Duration::from_secs(600),
                None,
                &gwt_core::index_coordinator::HeavyAttempt {
                    id: attempt.id(),
                    check_cancelled: &check,
                },
            )
            .unwrap();
        coordinator
            .reserve_heavy(
                &foreign,
                JobPriority::ManualRebuild,
                Duration::from_secs(600),
                None,
            )
            .unwrap();

        attempt
            .returned(Some("verify: deferred"), &coordinator)
            .unwrap();
        assert!(attempt.ensure_active().is_ok());
        let canceled = cancel_for_caller(
            repo.path(),
            "mine",
            attempt.id(),
            "superseded matrix",
            &authority(),
            &coordinator,
            &key,
        )
        .unwrap();
        assert_eq!(canceled.status, AttemptStatus::Interrupted);
        assert_eq!(
            canceled.reason.as_deref(),
            Some("caller cancellation: superseded matrix")
        );
        assert!(!coordinator.heavy_reservation_path(&key).exists());
        assert!(coordinator.heavy_reservation_path(&foreign).exists());
        assert!(attempt.cancelled().unwrap());
        attempt
            .returned(Some("verify: deferred"), &coordinator)
            .unwrap();
        assert!(attempt.ensure_active().is_err());
        assert_eq!(
            load(repo.path(), attempt.id()).unwrap().unwrap().status,
            AttemptStatus::Interrupted
        );
        assert!(attempt.mark_running("vrr-late").is_err());

        let next = begin_for_caller(
            repo.path(),
            "mine",
            &authority(),
            &key,
            &commands,
            "new-token",
        )
        .unwrap();
        assert_ne!(next.id(), attempt.id());
        assert!(!next.cancelled().unwrap());
    }

    #[test]
    fn cancellation_refuses_foreign_session_before_runtime_mutation() {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open(runtime.path()).unwrap();
        let key = TargetKey::verification("mine", "worktree");
        let attempt = begin_for_caller(
            repo.path(),
            "other-session",
            &authority(),
            &key,
            &[],
            "token",
        )
        .unwrap();
        let check = || attempt.cancelled().map_err(Into::into);
        coordinator
            .reserve_heavy_for_attempt(
                &key,
                JobPriority::ManualRebuild,
                Duration::from_secs(600),
                None,
                &gwt_core::index_coordinator::HeavyAttempt {
                    id: attempt.id(),
                    check_cancelled: &check,
                },
            )
            .unwrap();
        let before = std::fs::read(coordinator.heavy_reservation_path(&key)).unwrap();
        let error = cancel_for_caller(
            repo.path(),
            "mine",
            attempt.id(),
            "stop",
            &authority(),
            &coordinator,
            &key,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("not your verification attempt"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(coordinator.heavy_reservation_path(&key)).unwrap(),
            before
        );
        assert!(!attempt.cancelled().unwrap());
    }

    #[test]
    fn a_finalized_vrr_cannot_pass_before_its_exact_attempt_commits() {
        let _env = gwt_core::test_support::env_lock();
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(runtime.path());
        let coordinator = IndexCoordinator::open(runtime.path()).unwrap();
        let key = TargetKey::verification("mine", "worktree");
        let attempt =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &[], "token").unwrap();
        let commands = vec!["git --version".to_string()];
        let plan = super::super::register_plan_for_caller(
            repo.path(),
            "mine",
            commands.clone(),
            vec![],
            vec![],
            false,
            &authority(),
        )
        .unwrap();
        let (record, _) = super::super::run_verification(repo.path(), "mine", &commands).unwrap();
        assert_eq!(
            super::super::evaluate_evidence_snapshot(
                repo.path(),
                "mine",
                None,
                Some(&plan),
                &record
            ),
            super::super::EvidenceStatus::Fresh
        );
        let mut document = serde_json::to_value(record).unwrap();
        document["verification_attempt_id"] = attempt.id().into();
        let record: super::super::VerificationRunRecord = serde_json::from_value(document).unwrap();
        attempt.mark_running(&record.record_id).unwrap();
        super::super::save(repo.path(), &record).unwrap();
        let record = super::super::load(repo.path()).unwrap().unwrap();
        assert_eq!(
            super::super::evaluate_evidence_snapshot(
                repo.path(),
                "mine",
                None,
                Some(&plan),
                &record
            ),
            super::super::EvidenceStatus::Running
        );
        // A new pre-admission attempt cannot hide the exact incomplete binding.
        begin_for_caller(repo.path(), "mine", &authority(), &key, &[], "next-token").unwrap();
        assert_eq!(
            super::super::evaluate_evidence_snapshot(
                repo.path(),
                "mine",
                None,
                Some(&plan),
                &record
            ),
            super::super::EvidenceStatus::Running
        );
        // A cancel writer can die after its fence and before VRR settlement.
        let mut fenced = load(repo.path(), attempt.id()).unwrap().unwrap();
        fenced.status = AttemptStatus::Interrupted;
        fenced.reason = Some("caller cancellation: stop".to_string());
        save(repo.path(), &mut fenced).unwrap();
        trusted_store::with_write_lease(repo.path(), || {
            interruption::previous_external_terminations(repo.path(), None)
        })
        .unwrap();
        cancel_for_caller(
            repo.path(),
            "mine",
            attempt.id(),
            "stop",
            &authority(),
            &coordinator,
            &key,
        )
        .unwrap();
        let interrupted = super::super::load(repo.path()).unwrap().unwrap();
        assert!(!interrupted.all_passed);
        assert_eq!(
            interrupted.lifecycle.as_ref().unwrap().status,
            interruption::RunStatus::Interrupted
        );
        assert_eq!(
            interrupted
                .lifecycle
                .as_ref()
                .unwrap()
                .external_terminations,
            None,
            "a retried intentional cancellation must not spend termination budget"
        );
        settle_watchdog_with_coordinator(repo.path(), attempt.id(), "token", false, &coordinator)
            .unwrap();
        assert_eq!(
            super::super::load(repo.path()).unwrap().unwrap(),
            interrupted
        );
    }

    #[test]
    fn successor_recovers_reused_runner_attempt_and_its_reservation() {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(runtime.path());
        let coordinator = crate::cli::verification_lease::open_coordinator().unwrap();
        let key = TargetKey::verification("mine", "worktree");
        // Both the ordinary running record and the final VRR/control write gap
        // must settle after a restart without relying on the old watchdog.
        for with_lifecycle in [true, false] {
            let attempt =
                begin_for_caller(repo.path(), "predecessor", &authority(), &key, &[], "token")
                    .unwrap();
            let mut record = super::super::tests::passing_record("predecessor", "unused");
            record.format_version = Some(super::super::VERIFICATION_FORMAT_VERSION);
            if with_lifecycle {
                record.lifecycle = Some(interruption::RunLifecycle::running("token"));
            }
            let mut document = serde_json::to_value(record).unwrap();
            document["verification_attempt_id"] = attempt.id().into();
            let record = serde_json::from_value(document).unwrap();
            attempt.mark_running("vr-test").unwrap();
            let mut control = load(repo.path(), attempt.id()).unwrap().unwrap();
            let mut document = serde_json::to_value(&control).unwrap();
            document["runner_started_at"] =
                (crate::process::host_process_start_time(std::process::id()).unwrap() + 1).into();
            control = serde_json::from_value(document).unwrap();
            save(repo.path(), &mut control).unwrap();
            let check = || attempt.cancelled().map_err(Into::into);
            coordinator
                .reserve_heavy_for_attempt(
                    &key,
                    JobPriority::ManualRebuild,
                    Duration::from_secs(600),
                    None,
                    &gwt_core::index_coordinator::HeavyAttempt {
                        id: attempt.id(),
                        check_cancelled: &check,
                    },
                )
                .unwrap();
            super::super::save(repo.path(), &record).unwrap();

            let (completed, _) = super::super::run_verification(
                repo.path(),
                "successor",
                &["git --version".to_string()],
            )
            .unwrap();
            assert!(completed.all_passed);
            assert_eq!(
                load(repo.path(), attempt.id()).unwrap().unwrap().status,
                AttemptStatus::Interrupted
            );
            assert!(!coordinator.heavy_reservation_path(&key).exists());
        }
    }

    #[test]
    fn interrupted_final_write_retains_prior_external_termination_budget() {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(runtime.path());
        let coordinator = IndexCoordinator::open(runtime.path()).unwrap();
        let key = TargetKey::verification("mine", "worktree");
        let attempt =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &[], "token").unwrap();
        let mut record = super::super::tests::passing_record("mine", "unused");
        record.format_version = Some(super::super::VERIFICATION_FORMAT_VERSION);
        record.verified_head = Some("same-head".to_string());
        let mut document = serde_json::to_value(record).unwrap();
        document["verification_attempt_id"] = attempt.id().into();
        let record = serde_json::from_value(document).unwrap();
        attempt.mark_running("vr-test").unwrap();
        trusted_store::with_write_lease(repo.path(), || {
            attempt.retain_termination_budget_under_lease(Some(1))?;
            super::super::save(repo.path(), &record)
        })
        .unwrap();
        let active = trusted_store::with_write_lease(repo.path(), || {
            interruption::previous_external_terminations(repo.path(), Some("same-head"))
        })
        .unwrap_err();
        assert!(active
            .to_string()
            .contains("previous verification runner is unsettled"));
        // Both runner and watchdog died after final VRR save, before Completed.
        let mut fenced = load(repo.path(), attempt.id()).unwrap().unwrap();
        fenced.runner_pid = 0;
        save(repo.path(), &mut fenced).unwrap();
        let error = trusted_store::with_write_lease(repo.path(), || {
            interruption::previous_external_terminations(repo.path(), Some("same-head"))
        })
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("execution infrastructure failure"));
        let interrupted = super::super::load(repo.path()).unwrap().unwrap();
        assert_eq!(
            interrupted
                .lifecycle
                .as_ref()
                .unwrap()
                .external_terminations,
            Some(2)
        );
        settle_watchdog_with_coordinator(repo.path(), attempt.id(), "token", false, &coordinator)
            .unwrap();
        assert_eq!(
            super::super::load(repo.path()).unwrap().unwrap(),
            interrupted
        );
    }

    #[test]
    fn interrupted_watchdog_retries_cleanup_after_a_cancel_writer_dies() {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open(runtime.path()).unwrap();
        let key = TargetKey::verification("mine", "worktree");
        let attempt =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &[], "token").unwrap();
        let check = || attempt.cancelled().map_err(Into::into);
        coordinator
            .reserve_heavy_for_attempt(
                &key,
                JobPriority::ManualRebuild,
                Duration::from_secs(600),
                None,
                &gwt_core::index_coordinator::HeavyAttempt {
                    id: attempt.id(),
                    check_cancelled: &check,
                },
            )
            .unwrap();
        // The cancel writer published its fence but died before queue cleanup.
        let mut interrupted = load(repo.path(), attempt.id()).unwrap().unwrap();
        interrupted.status = AttemptStatus::Interrupted;
        interrupted.reason = Some("caller cancellation: superseded matrix".to_string());
        save(repo.path(), &mut interrupted).unwrap();
        assert!(coordinator.heavy_reservation_path(&key).exists());
        settle_watchdog_with_coordinator(repo.path(), attempt.id(), "token", false, &coordinator)
            .unwrap();
        assert!(!coordinator.heavy_reservation_path(&key).exists());
        assert_eq!(
            load(repo.path(), attempt.id()).unwrap().unwrap().reason,
            interrupted.reason
        );
    }

    #[test]
    fn authenticated_waiting_runner_death_releases_reservation_without_pid() {
        let repo = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let coordinator = IndexCoordinator::open(runtime.path()).unwrap();
        let key = TargetKey::verification("mine", "worktree");
        let attempt =
            begin_for_caller(repo.path(), "mine", &authority(), &key, &[], "token").unwrap();
        let check = || attempt.cancelled().map_err(Into::into);
        coordinator
            .reserve_heavy_for_attempt(
                &key,
                JobPriority::ManualRebuild,
                Duration::from_secs(600),
                None,
                &gwt_core::index_coordinator::HeavyAttempt {
                    id: attempt.id(),
                    check_cancelled: &check,
                },
            )
            .unwrap();
        settle_watchdog_with_coordinator(repo.path(), attempt.id(), "token", false, &coordinator)
            .unwrap();
        let interrupted = load(repo.path(), attempt.id()).unwrap().unwrap();
        assert_eq!(interrupted.status, AttemptStatus::Interrupted);
        assert!(interrupted.reason.unwrap().contains("external termination"));
        assert!(!coordinator.heavy_reservation_path(&key).exists());
    }
}
