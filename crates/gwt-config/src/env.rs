//! Thread-local environment overrides shared by every gwt crate (SPEC #4072).
//!
//! Environment variables are process-global: a test that rewrites one with
//! `std::env::set_var` is observed by every sibling test running in the same
//! binary, and by production readers racing with it (#3983, #4060). Tests pin
//! a value for the current thread with [`ScopedEnvOverride`] instead, and
//! production code reads through [`var_os`] / [`var`], which consult that
//! override before the process environment.
//!
//! One keyed mechanism backs every variable, so a new variable never needs
//! its own `thread_local!` or guard type. Production never installs an
//! override except where a caller pins a value for its own synchronous work
//! (for example hook materialization), so reads behave exactly like
//! `std::env::var_os` otherwise.

use std::{cell::RefCell, collections::HashMap, env::VarError, ffi::OsString, marker::PhantomData};

thread_local! {
    /// `key -> Some(value)` pins a value; `key -> None` hides the variable.
    static OVERRIDES: RefCell<HashMap<&'static str, Option<OsString>>> =
        RefCell::new(HashMap::new());
}

/// The current thread's override for `key`: `Some(Some(value))` when pinned,
/// `Some(None)` when hidden, `None` when the process environment applies.
pub fn override_for(key: &str) -> Option<Option<OsString>> {
    OVERRIDES.with(|overrides| overrides.borrow().get(key).cloned())
}

/// `std::env::var_os` that honours the current thread's override.
pub fn var_os(key: &str) -> Option<OsString> {
    match override_for(key) {
        Some(value) => value,
        None => std::env::var_os(key),
    }
}

/// `std::env::var` that honours the current thread's override.
pub fn var(key: &str) -> Result<String, VarError> {
    match var_os(key) {
        Some(value) => value.into_string().map_err(VarError::NotUnicode),
        None => Err(VarError::NotPresent),
    }
}

/// RAII guard pinning or hiding one variable for the current thread only.
///
/// Nested guards restore the previous override on drop. The guard is not
/// `Send`: the override lives in thread-local storage.
pub struct ScopedEnvOverride {
    key: &'static str,
    previous: Option<Option<OsString>>,
    _not_send: PhantomData<*const ()>,
}

impl ScopedEnvOverride {
    /// Pins `key` to `value` for the current thread.
    pub fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        Self::install(key, Some(value.into()))
    }

    /// Hides `key` for the current thread, as if it were unset.
    pub fn unset(key: &'static str) -> Self {
        Self::install(key, None)
    }

    fn install(key: &'static str, value: Option<OsString>) -> Self {
        let previous = OVERRIDES.with(|overrides| overrides.borrow_mut().insert(key, value));
        Self {
            key,
            previous,
            _not_send: PhantomData,
        }
    }
}

impl Drop for ScopedEnvOverride {
    fn drop(&mut self) {
        OVERRIDES.with(|overrides| {
            let mut overrides = overrides.borrow_mut();
            match self.previous.take() {
                Some(previous) => overrides.insert(self.key, previous),
                None => overrides.remove(self.key),
            };
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Keys no process ever sets, so the process-env fallback is deterministic.
    const KEY: &str = "GWT_SPEC_4072_OVERRIDE_PROBE";

    #[test]
    fn an_override_is_seen_only_by_its_own_thread_and_restores_on_drop() {
        assert_eq!(var_os(KEY), None);
        {
            let _pin = ScopedEnvOverride::set(KEY, "outer");
            assert_eq!(var(KEY).as_deref(), Ok("outer"));
            {
                let _nested = ScopedEnvOverride::set(KEY, "inner");
                assert_eq!(var(KEY).as_deref(), Ok("inner"));
                let seen_elsewhere = std::thread::spawn(|| var_os(KEY)).join().unwrap();
                assert_eq!(seen_elsewhere, None, "another thread must not see the pin");
            }
            assert_eq!(var(KEY).as_deref(), Ok("outer"));
        }
        assert_eq!(override_for(KEY), None);
        assert_eq!(var_os(KEY), None);
    }

    #[test]
    fn unset_hides_a_variable_the_process_environment_defines() {
        // Every process has PATH; hiding it must not touch the process env.
        let process_path = std::env::var_os("PATH");
        assert!(process_path.is_some());
        {
            let _hidden = ScopedEnvOverride::unset("PATH");
            assert_eq!(var_os("PATH"), None);
            assert_eq!(var("PATH"), Err(VarError::NotPresent));
            assert_eq!(std::env::var_os("PATH"), process_path);
        }
        assert_eq!(var_os("PATH"), process_path);
    }
}
