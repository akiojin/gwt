//! Resume the caller after admission, without holding capacity or executing
//! commands on its behalf. The authenticated attempt and pane route fence
//! notifications to their original Session and producing generation.

use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

use gwt_core::index_coordinator::{IndexCoordinator, VerificationDiskBudget};

use crate::cli::verification_lease;

const POLL: Duration = Duration::from_secs(5);
const MAX_WATCH: Duration = Duration::from_secs(3 * 60 * 60);

pub(super) fn run(worktree: &Path, id: &str, token: &str) -> io::Result<()> {
    run_resume(worktree, id, token, false)
}

pub(super) fn run_admitted(worktree: &Path, id: &str, token: &str) -> io::Result<()> {
    run_resume(worktree, id, token, true)
}

fn candidate(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
    admitted: bool,
) -> io::Result<Option<Vec<String>>> {
    if admitted {
        super::cancellation::admitted_resume_candidate(worktree, id, token, session)
    } else {
        super::cancellation::admission_resume_candidate(worktree, id, token, session)
    }
}

fn run_resume(worktree: &Path, id: &str, token: &str, admitted: bool) -> io::Result<()> {
    let Ok(session) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV) else {
        return Ok(());
    };
    if std::env::var_os(gwt_agent::GWT_PANE_WS_URL_ENV).is_none() {
        return Ok(());
    }
    let coordinator =
        crate::cli::verification_lease::open_coordinator().map_err(io::Error::other)?;
    let started = Instant::now();
    while started.elapsed() < MAX_WATCH {
        let waiting = poll_resume(
            worktree,
            id,
            token,
            &session,
            &coordinator,
            admitted,
            |command| command_disk_budgets(worktree, command),
            |commands| {
                let text = if admitted {
                    "[gwt] Your waiting verify.run has been admitted. Resume the unfinished task by inspecting verify.status and waiting for the existing tool result. Do not start another verify.run while this attempt is alive.".to_string()
                } else {
                    format!(
                "[gwt] Verification admission has capacity again. Continue the unfinished task in this Session: rerun verify.run with the identical full requested matrix and headed_e2e_commands. Requested commands: {}. A deferral is not completion; if admission defers again, retry or declare issue.monitor.wait with the explicit owner number. Do not start a duplicate of a live verify.run.",
                serde_json::to_string(commands).expect("commands serialize"),
                )
                };
                crate::cli::pane::notify_verification_resume(worktree, &session, &text, || {
                    candidate(worktree, id, token, &session, admitted)
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some(commands)
                })
                .map_err(io::Error::other)
            },
        )?;
        if !waiting {
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
    Ok(())
}

/// A tick has no side effects while capacity is full, or after cancellation,
/// replacement or lost authority. Return true only while observation is needed.
#[cfg(test)]
pub(super) fn poll(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
    coordinator: &IndexCoordinator,
    notify: impl FnMut(&[String]) -> io::Result<bool>,
) -> io::Result<bool> {
    poll_resume(
        worktree,
        id,
        token,
        session,
        coordinator,
        false,
        |_| Ok(Vec::new()),
        notify,
    )
}

#[cfg(test)]
pub(super) fn poll_with_disk_budgets(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
    coordinator: &IndexCoordinator,
    budgets: impl FnMut(&str) -> io::Result<Vec<VerificationDiskBudget>>,
    notify: impl FnMut(&[String]) -> io::Result<bool>,
) -> io::Result<bool> {
    poll_resume(
        worktree,
        id,
        token,
        session,
        coordinator,
        false,
        budgets,
        notify,
    )
}

fn command_disk_budgets(worktree: &Path, command: &str) -> io::Result<Vec<VerificationDiskBudget>> {
    let target = verification_lease::effective_cargo_target(worktree, command, false)
        .map_err(io::Error::other)?;
    verification_lease::command_admission_disk_budgets(worktree, command, target.as_deref())
        .map_err(io::Error::other)
}

#[allow(clippy::too_many_arguments)] // Keep the deferred disk probe and admitted notification on one fenced poll.
fn poll_resume(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
    coordinator: &IndexCoordinator,
    admitted: bool,
    mut budgets: impl FnMut(&str) -> io::Result<Vec<VerificationDiskBudget>>,
    mut notify: impl FnMut(&[String]) -> io::Result<bool>,
) -> io::Result<bool> {
    let Some(commands) = candidate(worktree, id, token, session, admitted)? else {
        return Ok(false);
    };
    if !admitted {
        if coordinator
            .heavy_pool_status()
            .map_err(io::Error::other)?
            .available
            == 0
        {
            return Ok(true);
        }
        for command in commands.iter().filter(|command| {
            verification_lease::classify_command(command)
                == verification_lease::CommandWeight::Heavy
        }) {
            // Fail closed on an unavailable disk observation: a free slot is
            // not proof that this matrix can pass admission.
            let observation = budgets(command).and_then(|budgets| {
                coordinator
                    .heavy_pool_status_with_disk_budgets(&budgets)
                    .map_err(io::Error::other)
            });
            match observation {
                Ok((pool, disks))
                    if pool.available > 0 && disks.iter().all(|disk| !disk.blocked) => {}
                Ok(_) => return Ok(true),
                Err(error) => {
                    tracing::debug!(%error, "verification resume disk admission observation unavailable");
                    return Ok(true);
                }
            }
        }
    }
    Ok(!notify(&commands)?)
}
