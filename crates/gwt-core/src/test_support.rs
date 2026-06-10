//! Test-only helpers shared across gwt crates (SPEC-3016 FR-003).
//!
//! Canonical home for process-global test machinery: [`env_lock`] serializes
//! tests that mutate environment variables and [`ScopedEnvVar`] restores an
//! environment variable when dropped. gwt-core unit tests reach this module
//! via `crate::test_support`; dependent crates enable the `test-support`
//! cargo feature from their dev-dependencies. gwt-only machinery (the fake
//! `gh` harness and CLI fixtures) stays in
//! `crates/gwt/src/cli/test_support.rs`.

use std::{
    ffi::OsString,
    sync::{Mutex, OnceLock},
};

/// Process-wide lock serializing tests that read or mutate environment
/// variables. Lock this before constructing a [`ScopedEnvVar`].
pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// RAII guard that sets or removes one environment variable and restores the
/// previous value on drop. Hold the [`env_lock`] mutex for the guard's whole
/// lifetime; the guard itself does not lock.
pub struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    /// Sets `key` to `value`, remembering the previous value for restore.
    pub fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    /// Removes `key`, remembering the previous value for restore.
    pub fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}
