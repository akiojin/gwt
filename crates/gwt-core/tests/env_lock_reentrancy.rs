//! Issue #4580: `env_lock()` must be reentrant.
//!
//! `env_lock()` serializes every test that reads or mutates the process-wide
//! environment. Before this regression guard existed it handed out a plain
//! `std::sync::Mutex`, which is not reentrant: a test body that held the lock
//! and then called a helper taking it again deadlocked on the spot.
//!
//! The lock is process-global, so these checks need their own test binary.
//! Every acquisition runs on a worker thread behind [`BUDGET`] so a
//! reintroduced deadlock fails the test instead of hanging CI.

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, RecvTimeoutError},
    Arc, Barrier, PoisonError,
};
use std::thread;
use std::time::Duration;

use gwt_core::test_support::env_lock;

/// Upper bound for one scenario. Exceeding it means the lock deadlocked, which
/// must surface as a failure rather than a hang. Generous enough for a loaded
/// CI runner and far above the 100ms test-hygiene floor.
const BUDGET: Duration = Duration::from_millis(10_000);

/// Runs `body` on a worker thread and fails if it neither finishes nor panics
/// within [`BUDGET`].
fn run_bounded(what: &str, body: impl FnOnce() + Send + 'static) {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        body();
        let _ = tx.send(());
    });
    match rx.recv_timeout(BUDGET) {
        Ok(()) => {}
        Err(RecvTimeoutError::Timeout) => {
            panic!("{what} did not finish within {BUDGET:?}: env_lock() deadlocked")
        }
        Err(RecvTimeoutError::Disconnected) => {
            panic!("{what} panicked on its worker thread; see the panic above")
        }
    }
}

#[test]
fn reentrant_acquisition_on_the_same_thread_does_not_deadlock() {
    run_bounded("a reentrant env_lock() acquisition", || {
        let outer = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
        let inner = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
        let innermost = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
        drop(innermost);
        drop(inner);
        drop(outer);

        // The depth bookkeeping must be back to zero: a fresh acquisition from
        // the same thread still has to succeed afterwards.
        drop(env_lock().lock().unwrap_or_else(PoisonError::into_inner));
    });
}

#[test]
fn concurrent_threads_are_still_mutually_excluded() {
    run_bounded("the cross-thread exclusion sweep", || {
        const THREADS: usize = 8;
        const ROUNDS: usize = 64;

        let inside = Arc::new(AtomicUsize::new(0));
        let violations = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(THREADS));

        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                let inside = Arc::clone(&inside);
                let violations = Arc::clone(&violations);
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    for _ in 0..ROUNDS {
                        let guard = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
                        if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                            violations.fetch_add(1, Ordering::SeqCst);
                        }
                        thread::yield_now();
                        if inside.fetch_sub(1, Ordering::SeqCst) != 1 {
                            violations.fetch_add(1, Ordering::SeqCst);
                        }
                        drop(guard);
                    }
                })
            })
            .collect();

        for worker in workers {
            worker.join().expect("exclusion worker");
        }

        assert_eq!(
            violations.load(Ordering::SeqCst),
            0,
            "two threads observed themselves inside the env_lock() critical section at once"
        );
    });
}

#[test]
fn a_nested_hold_still_excludes_other_threads() {
    run_bounded("the nested-hold exclusion handshake", || {
        let both_running = Arc::new(Barrier::new(2));
        let contender_attempted = Arc::new(AtomicBool::new(false));
        let holder_released = Arc::new(AtomicBool::new(false));

        let holder = {
            let both_running = Arc::clone(&both_running);
            let contender_attempted = Arc::clone(&contender_attempted);
            let holder_released = Arc::clone(&holder_released);
            thread::spawn(move || {
                let outer = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
                let nested = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
                both_running.wait();
                while !contender_attempted.load(Ordering::SeqCst) {
                    thread::yield_now();
                }
                holder_released.store(true, Ordering::SeqCst);
                drop(nested);
                drop(outer);
            })
        };

        let contender = {
            let both_running = Arc::clone(&both_running);
            let contender_attempted = Arc::clone(&contender_attempted);
            let holder_released = Arc::clone(&holder_released);
            thread::spawn(move || {
                both_running.wait();
                contender_attempted.store(true, Ordering::SeqCst);
                let guard = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
                assert!(
                    holder_released.load(Ordering::SeqCst),
                    "another thread acquired env_lock() while it was held reentrantly"
                );
                drop(guard);
            })
        };

        holder.join().expect("nested holder");
        contender.join().expect("nested contender");
    });
}

#[test]
fn poisoning_behaves_exactly_as_it_did_before() {
    run_bounded("the poisoning handshake", || {
        let poisoner = thread::spawn(|| {
            let _guard = env_lock().lock().unwrap_or_else(PoisonError::into_inner);
            panic!("intentional panic: poisons env_lock() for this test binary");
        });
        assert!(
            poisoner.join().is_err(),
            "the poisoning worker must have panicked"
        );

        let outer = env_lock().lock();
        assert!(
            outer.is_err(),
            "a panic while holding env_lock() must still poison it"
        );
        let outer = outer.unwrap_or_else(PoisonError::into_inner);

        let nested = env_lock().lock();
        assert!(
            nested.is_err(),
            "a nested acquisition must report the same poison state as the outer one"
        );
        drop(nested.unwrap_or_else(PoisonError::into_inner));
        drop(outer);
    });
}
