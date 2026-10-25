//! Named wall-clock budgets that a test pins explicitly (SPEC #4740).
//!
//! A production deadline such as "the prefs transaction gets 250 ms" used to
//! be a bare `const`. A test that asserted on the transaction's *outcome* then
//! asserted on how fast the runner happened to be that minute, and a loaded CI
//! host turned a passing state machine into a failing one. Ten separate
//! flakes had that shape.
//!
//! Route such a deadline through a [`DeadlineBudget`] instead. Production
//! resolves the default. Under `cfg(test)` / the `test-support` feature a
//! budget resolves, in order:
//!
//! 1. a thread-scoped pin (`ScopedDeadlineBudget`) — exact for a test whose
//!    deadline is read on the test thread;
//! 2. a process-wide pin, the environment variable
//!    `GWT_TEST_BUDGET_<NAME>_MS` — for a deadline read on another thread
//!    (take `test_support::env_lock()` first, as for any env mutation);
//! 3. **load mode**: when [`LOAD_MODE_ENV`] is `1`, every unpinned budget
//!    resolves to [`LOAD_MODE_BUDGET`] (zero: spent on arrival), so a test
//!    that silently leans on the default fails deterministically instead of
//!    once a week on CI. Zero rather than "very short": a 1 ms budget still
//!    races the work it bounds, and a race only exposes the dependency
//!    sometimes;
//! 4. the production default.
//!
//! A test that is not about the deadline pins [`HANG_GUARD`]: the outcome no
//! longer depends on elapsed time, and the deadline only keeps a genuinely
//! wedged test from hanging. A test that *is* about the deadline pins the
//! short value it asserts on.
//!
//! **Child processes inherit load mode, deliberately.** A test that spawns
//! the real `gwtd` exercises production code in the child, and a budget
//! dependency there is as real as one on the test thread; stripping
//! [`LOAD_MODE_ENV`] from children would hide it. A thread pin cannot cross
//! a process boundary, so such a test pins the child's budget explicitly
//! through [`DeadlineBudget::env_var`] on the child's command.

use std::time::Duration;

/// Environment switch for load mode. `1` shrinks every unpinned budget.
pub const LOAD_MODE_ENV: &str = "GWT_TEST_SHRINK_BUDGETS";

/// What an unpinned budget resolves to in load mode: already spent.
pub const LOAD_MODE_BUDGET: Duration = Duration::ZERO;

/// The pin for a test whose verdict must not depend on elapsed time. Long
/// enough that reaching it means the test is wedged, not that the host is
/// slow; still finite so `Instant + budget` never overflows.
pub const HANG_GUARD: Duration = Duration::from_secs(600);

/// A named production deadline. `name` is upper snake case; it forms the
/// `GWT_TEST_BUDGET_<NAME>_MS` override variable.
#[derive(Debug, Clone, Copy)]
pub struct DeadlineBudget {
    name: &'static str,
    default: Duration,
}

impl DeadlineBudget {
    pub const fn new(name: &'static str, default: Duration) -> Self {
        Self { name, default }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The production value, regardless of pins or load mode.
    pub const fn default_value(&self) -> Duration {
        self.default
    }

    /// The deadline to enforce now.
    pub fn resolve(&self) -> Duration {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(value) = test_hooks::resolve(self) {
            return value;
        }
        self.default
    }

    /// The process-wide override variable for this budget.
    pub fn env_var(&self) -> String {
        format!("GWT_TEST_BUDGET_{}_MS", self.name)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use test_hooks::ScopedDeadlineBudget;

#[cfg(any(test, feature = "test-support"))]
mod test_hooks {
    use std::{cell::RefCell, marker::PhantomData, rc::Rc, time::Duration};

    use super::{DeadlineBudget, LOAD_MODE_BUDGET, LOAD_MODE_ENV};

    thread_local! {
        static PINS: RefCell<Vec<(&'static str, Duration)>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn resolve(budget: &DeadlineBudget) -> Option<Duration> {
        let pinned = PINS.with(|pins| {
            pins.borrow()
                .iter()
                .rev()
                .find(|(name, _)| *name == budget.name)
                .map(|(_, value)| *value)
        });
        if pinned.is_some() {
            return pinned;
        }
        if let Some(ms) = std::env::var_os(budget.env_var())
            .and_then(|value| value.to_string_lossy().trim().parse::<u64>().ok())
        {
            return Some(Duration::from_millis(ms));
        }
        if std::env::var_os(LOAD_MODE_ENV).is_some_and(|value| value == "1") {
            return Some(LOAD_MODE_BUDGET);
        }
        None
    }

    /// Pin a budget on the current thread until dropped. Pins nest; the
    /// innermost wins.
    #[must_use = "the pin is released when the guard drops"]
    pub struct ScopedDeadlineBudget {
        name: &'static str,
        _thread: PhantomData<Rc<()>>,
    }

    impl ScopedDeadlineBudget {
        pub fn pin(budget: &DeadlineBudget, value: Duration) -> Self {
            PINS.with(|pins| pins.borrow_mut().push((budget.name, value)));
            Self {
                name: budget.name,
                _thread: PhantomData,
            }
        }

        /// Pin [`super::HANG_GUARD`]: the test's verdict must not depend on
        /// how long the budgeted work takes.
        pub fn hang_guard(budget: &DeadlineBudget) -> Self {
            Self::pin(budget, super::HANG_GUARD)
        }
    }

    impl Drop for ScopedDeadlineBudget {
        fn drop(&mut self) {
            PINS.with(|pins| {
                let mut pins = pins.borrow_mut();
                if let Some(index) = pins.iter().rposition(|(name, _)| *name == self.name) {
                    pins.remove(index);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, ScopedEnvVar};

    const BUDGET: DeadlineBudget =
        DeadlineBudget::new("DEADLINE_BUDGET_SELF_TEST", Duration::from_millis(250));

    #[test]
    fn unpinned_budget_resolves_the_production_default() {
        let _env = env_lock().lock().unwrap();
        let _load = ScopedEnvVar::unset(LOAD_MODE_ENV);
        assert_eq!(BUDGET.resolve(), Duration::from_millis(250));
    }

    #[test]
    fn load_mode_shrinks_an_unpinned_budget() {
        let _env = env_lock().lock().unwrap();
        let _load = ScopedEnvVar::set(LOAD_MODE_ENV, "1");
        assert_eq!(BUDGET.resolve(), LOAD_MODE_BUDGET);
    }

    #[test]
    fn thread_pin_wins_over_load_mode_and_is_released_on_drop() {
        let _env = env_lock().lock().unwrap();
        let _load = ScopedEnvVar::set(LOAD_MODE_ENV, "1");
        {
            let _outer = ScopedDeadlineBudget::hang_guard(&BUDGET);
            assert_eq!(BUDGET.resolve(), HANG_GUARD);
            {
                let _inner = ScopedDeadlineBudget::pin(&BUDGET, Duration::from_millis(150));
                assert_eq!(BUDGET.resolve(), Duration::from_millis(150));
            }
            assert_eq!(BUDGET.resolve(), HANG_GUARD);
        }
        assert_eq!(BUDGET.resolve(), LOAD_MODE_BUDGET);
    }

    #[test]
    fn thread_pin_does_not_leak_to_other_threads() {
        let _env = env_lock().lock().unwrap();
        let _load = ScopedEnvVar::unset(LOAD_MODE_ENV);
        let _pin = ScopedDeadlineBudget::hang_guard(&BUDGET);
        let other = std::thread::spawn(|| BUDGET.resolve()).join().unwrap();
        assert_eq!(other, Duration::from_millis(250));
    }

    #[test]
    fn env_pin_reaches_other_threads_and_wins_over_load_mode() {
        let _env = env_lock().lock().unwrap();
        let _load = ScopedEnvVar::set(LOAD_MODE_ENV, "1");
        let _pin = ScopedEnvVar::set("GWT_TEST_BUDGET_DEADLINE_BUDGET_SELF_TEST_MS", "2000");
        let other = std::thread::spawn(|| BUDGET.resolve()).join().unwrap();
        assert_eq!(other, Duration::from_millis(2000));
    }
}
