use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gwt_core::paths::{ensure_dir, gwt_logs_dir};

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn log(message: impl AsRef<str>) {
    let Some(path) = configured_log_path() else {
        return;
    };
    append_line(&path, message.as_ref());
}

pub(crate) fn log_lazy(message: impl FnOnce() -> String) {
    let Some(path) = configured_log_path() else {
        return;
    };
    append_line(&path, &message());
}

#[allow(dead_code)]
pub(crate) fn is_enabled() -> bool {
    configured_log_path().is_some()
}

fn append_line(path: &Path, message: &str) {
    let Some(parent) = path.parent() else {
        return;
    };
    if ensure_dir(parent).is_err() {
        return;
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let _ = writeln!(file, "[{timestamp_ms}] {message}");
}

#[cfg(test)]
fn configured_log_path() -> Option<PathBuf> {
    configured_log_path_uncached()
}

#[cfg(not(test))]
fn configured_log_path() -> Option<PathBuf> {
    static CONFIGURED_LOG_PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CONFIGURED_LOG_PATH
        .get_or_init(configured_log_path_uncached)
        .clone()
}

fn configured_log_path_uncached() -> Option<PathBuf> {
    if let Some(path) = explicit_log_path() {
        return Some(path);
    }
    if scroll_debug_enabled() {
        return Some(gwt_logs_dir().join("scroll-debug.log"));
    }
    None
}

fn explicit_log_path() -> Option<PathBuf> {
    read_env_os("GWT_SCROLL_DEBUG_LOG")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn scroll_debug_enabled() -> bool {
    matches!(
        read_env_os("GWT_SCROLL_DEBUG")
            .as_deref()
            .and_then(std::ffi::OsStr::to_str),
        Some("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

fn read_env_os(key: &str) -> Option<OsString> {
    #[cfg(test)]
    if let Some(value) = test_env_value(key) {
        return value;
    }
    std::env::var_os(key)
}

#[cfg(test)]
fn test_env_value(key: &str) -> Option<Option<OsString>> {
    test_overrides()
        .lock()
        .expect("scroll debug test env lock")
        .iter()
        .find_map(|(candidate, value)| (candidate == key).then(|| value.clone()))
}

#[cfg(test)]
fn test_overrides() -> &'static std::sync::Mutex<Vec<(String, Option<OsString>)>> {
    type TestOverrides = std::sync::Mutex<Vec<(String, Option<OsString>)>>;

    static TEST_OVERRIDES: std::sync::OnceLock<TestOverrides> = std::sync::OnceLock::new();
    TEST_OVERRIDES.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[cfg(test)]
fn with_test_env<T>(entries: &[(&str, Option<&str>)], run: impl FnOnce() -> T) -> T {
    let mut overrides = test_overrides().lock().expect("scroll debug test env lock");
    let previous = std::mem::take(&mut *overrides);
    overrides.extend(entries.iter().map(|(key, value)| {
        (
            (*key).to_string(),
            value.map(|value| OsString::from(value.to_string())),
        )
    }));
    drop(overrides);
    let result = run();
    *test_overrides().lock().expect("scroll debug test env lock") = previous;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    static SCROLL_DEBUG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_log_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("gwt-scroll-debug-{name}-{unique}.log"))
    }

    #[test]
    fn explicit_log_path_env_takes_precedence() {
        let _guard = SCROLL_DEBUG_TEST_LOCK
            .lock()
            .expect("scroll debug test lock");
        let expected = temp_log_path("explicit");
        let actual = with_test_env(
            &[
                ("GWT_SCROLL_DEBUG", Some("1")),
                (
                    "GWT_SCROLL_DEBUG_LOG",
                    Some(expected.to_string_lossy().as_ref()),
                ),
            ],
            configured_log_path,
        )
        .expect("configured log path");
        assert_eq!(actual, expected);
    }

    #[test]
    fn configured_log_path_uses_default_scroll_debug_file_when_enabled() {
        let _guard = SCROLL_DEBUG_TEST_LOCK
            .lock()
            .expect("scroll debug test lock");
        let actual = with_test_env(
            &[
                ("GWT_SCROLL_DEBUG", Some("1")),
                ("GWT_SCROLL_DEBUG_LOG", None),
            ],
            configured_log_path,
        )
        .expect("configured log path");
        assert_eq!(actual, gwt_logs_dir().join("scroll-debug.log"));
    }

    #[test]
    fn log_uses_explicit_path_when_provided() {
        let _guard = SCROLL_DEBUG_TEST_LOCK
            .lock()
            .expect("scroll debug test lock");
        let target = temp_log_path("explicit-write");
        let _ = std::fs::remove_file(&target);

        with_test_env(
            &[(
                "GWT_SCROLL_DEBUG_LOG",
                Some(target.to_string_lossy().as_ref()),
            )],
            || log("event=mouse kind=ScrollUp"),
        );

        let contents = std::fs::read_to_string(&target).expect("explicit debug log contents");
        assert!(contents.contains("event=mouse"));
        assert!(contents.contains("kind=ScrollUp"));
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn log_lazy_skips_message_construction_when_disabled() {
        let _guard = SCROLL_DEBUG_TEST_LOCK
            .lock()
            .expect("scroll debug test lock");
        let called = Cell::new(false);

        with_test_env(
            &[("GWT_SCROLL_DEBUG", None), ("GWT_SCROLL_DEBUG_LOG", None)],
            || {
                log_lazy(|| {
                    called.set(true);
                    "event=disabled".to_string()
                });
            },
        );

        assert!(
            !called.get(),
            "disabled debug logging should not evaluate lazy message construction"
        );
    }
}
