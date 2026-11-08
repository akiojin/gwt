//! Resume the caller after admission, without holding capacity or executing
//! commands on its behalf. The authenticated attempt and pane route fence
//! notifications to their original Session and producing generation.

use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

use gwt_core::index_coordinator::IndexCoordinator;

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
    poll_resume(worktree, id, token, session, coordinator, false, notify)
}

fn poll_resume(
    worktree: &Path,
    id: &str,
    token: &str,
    session: &str,
    coordinator: &IndexCoordinator,
    admitted: bool,
    mut notify: impl FnMut(&[String]) -> io::Result<bool>,
) -> io::Result<bool> {
    let Some(commands) = candidate(worktree, id, token, session, admitted)? else {
        return Ok(false);
    };
    if !admitted
        && coordinator
            .heavy_pool_status()
            .map_err(io::Error::other)?
            .available
            == 0
    {
        return Ok(true);
    }
    Ok(!notify(&commands)?)
}
