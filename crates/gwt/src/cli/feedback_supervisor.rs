//! Windows-only owner for one fixed, inherited-environment feedback sender.

use std::{
    io,
    thread::{self, JoinHandle},
};

#[cfg(windows)]
use std::{
    fs::File,
    os::windows::io::AsHandle,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(windows)]
use gwt_core::{
    feedback::{
        auto_report::{AutoReason, OwnedAutoReason, OwnedCleanupObservation, OwnedExitObservation},
        FeedbackState,
    },
    process_tree::WindowsJobObject,
};
#[cfg(windows)]
use uuid::Uuid;

#[cfg(windows)]
use super::{
    feedback,
    feedback_owner_bootstrap::{self as bootstrap, Bootstrap, Clock, ReceivedBootstrap, Role},
};

#[cfg(windows)]
pub(super) fn spawn_owner(root: &Path, id: &str) -> io::Result<()> {
    // Capture the shared origin before permission for the first process spawn.
    let clock = Clock::capture()?;
    let descriptor = Bootstrap::supervisor(
        Uuid::parse_str(id).map_err(|_| failure())?,
        Uuid::new_v4(),
        Uuid::new_v4(),
        clock,
    );
    let owner = bootstrap::spawn_private(root, &descriptor, None)?;
    // Ack loss/child exit is not cancellation, failure-before-spawn, or Sent.
    // Never launch a fallback or kill an owner whose acceptance is unobserved.
    let _ = owner.wait_for_ack();
    Ok(())
}

#[cfg(windows)]
pub(super) fn run(argv: &[String]) -> i32 {
    let role = match argv.get(1).map(String::as_str) {
        Some("--gwt-feedback-supervisor") => Role::Supervisor,
        Some("--gwt-feedback-sender") => Role::Sender,
        _ => return 1,
    };
    let result = (|| {
        let received = bootstrap::receive(argv, role)?;
        let root = std::env::current_dir()?;
        match role {
            Role::Supervisor => supervise(root, received),
            Role::Sender => feedback::run_owned_sender(&root, &received),
        }
    })();
    i32::from(result.is_err())
}

fn failure() -> io::Error {
    io::Error::other("automatic feedback ownership unavailable")
}

// Only one such job can exist at a time. The driver never waits for unfinished
// filesystem I/O and never transfers its mutable process/Job handles to it.
fn bookkeep<T: Send + 'static>(
    operation: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<JoinHandle<io::Result<T>>> {
    thread::Builder::new()
        .name("feedback-receipt".into())
        .spawn(operation)
}

fn take_finished<T>(job: &mut Option<JoinHandle<io::Result<T>>>) -> Option<io::Result<T>> {
    if !job.as_ref()?.is_finished() {
        return None;
    }
    Some(
        job.take()
            .expect("finished job")
            .join()
            .unwrap_or_else(|_| Err(failure())),
    )
}

fn await_bookkeeping<T>(
    job: &mut Option<JoinHandle<io::Result<T>>>,
    mut poll_deadline: impl FnMut() -> io::Result<()>,
) -> io::Result<T> {
    loop {
        if let Some(result) = take_finished(job) {
            return result;
        }
        poll_deadline()?;
    }
}

#[cfg(windows)]
fn pause(clock: Clock) -> io::Result<()> {
    thread::sleep(clock.remaining_exit()?.min(Duration::from_millis(100)));
    Ok(())
}

#[cfg(windows)]
fn authorize(root: &Path, clock: Clock) -> io::Result<()> {
    clock.remaining_work()?;
    if !matches!(feedback::auto_policy(root), Ok(true)) {
        return Err(failure());
    }
    clock.remaining_work()?;
    Ok(())
}

#[cfg(windows)]
fn supervise(root: PathBuf, received: ReceivedBootstrap) -> io::Result<()> {
    let descriptor = received.descriptor.clone();
    let id = descriptor.receipt_id.to_string();
    let attempt = descriptor.attempt_id;
    let clock = descriptor.clock;
    let initial_root = root.clone();
    let initial_id = id.clone();
    let mut registration = Some(bookkeep(move || {
        let store = feedback::owned_store(&initial_root);
        let owner = store.try_owner_lock(&initial_id)?;
        store.begin_owned_attempt(&initial_id, attempt, || authorize(&initial_root, clock))?;
        Ok(owner)
    })?);
    let _owner: File = await_bookkeeping(&mut registration, || pause(clock))?;
    // Durable registration is responsibility acceptance, not delivery.
    let _ = received.acknowledge();
    if clock.remaining_work().is_err() {
        return settle_without_spawn(root, id, attempt, clock);
    }
    let mut job = match WindowsJobObject::new() {
        Ok(job) => job,
        Err(_) => return settle_without_spawn(root, id, attempt, clock),
    };
    let owner = match bootstrap::current_owner_handle() {
        Ok(owner) => owner,
        Err(_) => return settle_without_spawn(root, id, attempt, clock),
    };
    let identity = match bootstrap::creation_identity(owner.as_handle()) {
        Ok(identity) => identity,
        Err(_) => return settle_without_spawn(root, id, attempt, clock),
    };
    let sender_descriptor = descriptor.sender(identity);
    let mut sender =
        match bootstrap::spawn_private(&root, &sender_descriptor, Some(owner.as_handle())) {
            Ok(sender) => sender,
            Err(_) => {
                // A failed native spawn can follow creation of a suspended child.
                // No exact exit observation came back: never invent sender0.
                return settle_unknown_start(root, id, attempt, clock);
            }
        };
    let started = job
        .assign_suspended_handle(sender.child.process_handle())
        .is_ok()
        && sender.child.resume().is_ok();
    let mut stopping = !started;
    let mut exit_confirmed = false;
    let mut cleanup_confirmed = false;
    let mut cancellation_done = false;
    let mut observation_started = false;
    let mut bookkeeping = None;
    if stopping {
        let _ = job.request_termination();
        let _ = sender.child.request_termination();
    }
    loop {
        if let Some(result) = take_finished(&mut bookkeeping) {
            result?;
            if observation_started {
                return Ok(());
            }
            cancellation_done = true;
        }
        if !exit_confirmed {
            exit_confirmed = matches!(sender.child.try_wait(), Ok(Some(_)));
        }
        let deadline = clock.remaining_work().is_err();
        if !stopping && (exit_confirmed || deadline) {
            stopping = true;
            let _ = job.request_termination();
            if !exit_confirmed {
                let _ = sender.child.request_termination();
            }
        }
        if stopping && bookkeeping.is_none() && !cancellation_done {
            let saved_root = root.clone();
            let saved_id = id.clone();
            let reason = if !started {
                OwnedAutoReason::WorkerUnavailable
            } else if deadline {
                OwnedAutoReason::WorkDeadline
            } else {
                OwnedAutoReason::SenderInterrupted
            };
            bookkeeping = Some(bookkeep(move || {
                let store = feedback::owned_store(&saved_root);
                // An already persisted remote success/failure is authoritative.
                if store.read_auto(&saved_id)?.state == FeedbackState::Sending {
                    store.cancel_owned_attempt(&saved_id, attempt, reason)?;
                    store.finish_owned_attempt(
                        &saved_id,
                        attempt,
                        Err(if reason == OwnedAutoReason::WorkDeadline {
                            AutoReason::PreSubmitFailed
                        } else {
                            AutoReason::SenderInterrupted
                        }),
                    )?;
                }
                Ok(())
            })?);
        }
        if stopping && exit_confirmed {
            cleanup_confirmed = matches!(job.termination_confirmed(), Ok(true));
        }
        if cancellation_done && exit_confirmed && cleanup_confirmed && !observation_started {
            let saved_root = root.clone();
            let saved_id = id.clone();
            bookkeeping = Some(bookkeep(move || {
                feedback::owned_store(&saved_root).observe_owned_exit(
                    &saved_id,
                    attempt,
                    OwnedExitObservation::Confirmed,
                    OwnedCleanupObservation::Confirmed,
                    None,
                )?;
                Ok(())
            })?);
            observation_started = true;
        }
        if cancellation_done
            && !observation_started
            && clock
                .remaining_exit()
                .is_ok_and(|remaining| remaining <= Duration::from_millis(100))
        {
            let saved_root = root.clone();
            let saved_id = id.clone();
            bookkeeping = Some(bookkeep(move || {
                feedback::owned_store(&saved_root).observe_owned_exit(
                    &saved_id,
                    attempt,
                    if exit_confirmed {
                        OwnedExitObservation::Confirmed
                    } else {
                        OwnedExitObservation::Unknown
                    },
                    OwnedCleanupObservation::Unknown,
                    Some(OwnedAutoReason::CleanupUnobserved),
                )?;
                Ok(())
            })?);
            observation_started = true;
        }
        if clock.remaining_exit().is_err() {
            let _ = job.request_termination();
            let _ = sender.child.request_termination();
            // Initial Unknown observations remain durable if a final save is
            // blocked. Returning terminates this private process and its I/O
            // thread; it does not assert exit/cleanup or permit another send.
            return Err(failure());
        }
        pause(clock)?;
    }
}

#[cfg(windows)]
fn settle_without_spawn(root: PathBuf, id: String, attempt: Uuid, clock: Clock) -> io::Result<()> {
    let mut task = Some(bookkeep(move || {
        let store = feedback::owned_store(&root);
        store.cancel_owned_attempt(&id, attempt, OwnedAutoReason::WorkerUnavailable)?;
        store.finish_owned_attempt(&id, attempt, Err(AutoReason::WorkerUnavailable))?;
        store.observe_owned_exit(
            &id,
            attempt,
            OwnedExitObservation::NotStarted,
            OwnedCleanupObservation::Confirmed,
            None,
        )?;
        Ok(())
    })?);
    await_bookkeeping(&mut task, || pause(clock))
}

#[cfg(windows)]
fn settle_unknown_start(root: PathBuf, id: String, attempt: Uuid, clock: Clock) -> io::Result<()> {
    let mut task = Some(bookkeep(move || {
        let store = feedback::owned_store(&root);
        store.cancel_owned_attempt(&id, attempt, OwnedAutoReason::WorkerUnavailable)?;
        store.finish_owned_attempt(&id, attempt, Err(AutoReason::RemoteOutcomeUnknown))?;
        Ok(())
    })?);
    await_bookkeeping(&mut task, || pause(clock))
}

#[cfg(test)]
mod tests {
    use super::super::feedback_owner_bootstrap::Clock;
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn blocked_bookkeeping_returns_at_the_original_deadline_without_joining() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut job = Some(
            bookkeep(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(7)
            })
            .unwrap(),
        );
        started_rx.recv().unwrap();
        let clock = Clock {
            d0_ticks: 1_000,
            frequency: 1_000,
        };
        let mut polls = 0;
        let result = await_bookkeeping(&mut job, || {
            polls += 1;
            let now = if polls == 1 { 120_000 } else { 121_000 };
            clock.remaining_exit_at(now, 1_000).map(|_| ())
        });
        let remained_pending = job.as_ref().is_some_and(|job| !job.is_finished());
        release_tx.send(()).unwrap();
        let completed = job.take().unwrap().join().unwrap().unwrap();
        assert!(result.is_err());
        assert!(remained_pending);
        assert_eq!(polls, 2);
        assert_eq!(completed, 7);
    }

    #[test]
    fn completed_bookkeeping_is_returned_without_polling_the_deadline() {
        let mut job = Some(bookkeep(|| Ok(7)).unwrap());
        while !job.as_ref().unwrap().is_finished() {
            thread::yield_now();
        }
        assert_eq!(await_bookkeeping(&mut job, || Err(failure())).unwrap(), 7);
        assert!(job.is_none());
    }
}
