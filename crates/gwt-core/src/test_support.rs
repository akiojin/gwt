//! Test-only helpers shared across gwt crates (SPEC-3016 FR-003).
//!
//! Canonical home for process-global test machinery: [`env_lock`] serializes
//! tests that mutate environment variables, [`ScopedEnvVar`] restores an
//! environment variable when dropped, and [`ScopedGwtHome`] isolates gwt home
//! path resolution without process-wide `HOME` mutation. gwt-core unit tests
//! reach this module via `crate::test_support`; dependent crates enable the
//! `test-support` cargo feature from their dev-dependencies. gwt-only
//! machinery (the fake `gh` harness and CLI fixtures) stays in
//! `crates/gwt/src/cli/test_support.rs`.

use std::{
    cell::RefCell,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// Process-wide lock serializing tests that read or mutate environment
/// variables. Lock this before constructing a [`ScopedEnvVar`].
pub fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

thread_local! {
    static GWT_HOME_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Returns the thread-local gwt home override used by tests.
pub fn gwt_home_override() -> Option<PathBuf> {
    GWT_HOME_OVERRIDE.with(|value| value.borrow().clone())
}

/// RAII guard that overrides the gwt home root for the current test thread.
///
/// Prefer this over mutating `HOME` for in-process tests. Environment
/// variables are process-global, so changing them in one parallel test can
/// make unrelated tests write into the real user home.
pub struct ScopedGwtHome {
    previous: Option<PathBuf>,
}

impl ScopedGwtHome {
    pub fn set(path: impl AsRef<Path>) -> Self {
        let next = path.as_ref().to_path_buf();
        let previous = GWT_HOME_OVERRIDE.with(|value| value.replace(Some(next)));
        Self { previous }
    }
}

impl Drop for ScopedGwtHome {
    fn drop(&mut self) {
        GWT_HOME_OVERRIDE.with(|value| {
            value.replace(self.previous.take());
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_gwt_home_is_thread_local_and_restores() {
        assert!(gwt_home_override().is_none());
        let home = std::env::temp_dir().join("gwt-test-home-override");

        {
            let _guard = ScopedGwtHome::set(&home);
            assert_eq!(gwt_home_override().as_deref(), Some(home.as_path()));
            std::thread::spawn(|| {
                assert!(gwt_home_override().is_none());
            })
            .join()
            .expect("thread");
        }

        assert!(gwt_home_override().is_none());
    }
}
