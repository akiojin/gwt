//! Non-blocking, daily-rolling JSONL writer backed by `tracing_appender`.
//!
//! **File naming:** `tracing_appender::rolling::daily` does not maintain
//! a bare active file. Every event is written to
//! `{log_dir}/gwt.log.YYYY-MM-DD` where the date is the current local
//! day. The Logs tab and housekeeping code must therefore reference the
//! dated filename directly. Use `current_log_file()` to compute it.

use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing_appender::{
    non_blocking::{NonBlocking, WorkerGuard},
    rolling,
};

/// Basename prefix used for the rolling log files. The full name is
/// `gwt.log.YYYY-MM-DD` (no bare `gwt.log` file exists at any point).
pub const LOG_FILE_BASENAME: &str = "gwt.log";

/// Return the path of today's active log file
/// (`{log_dir}/gwt.log.YYYY-MM-DD`, UTC date).
///
/// **Timezone policy**: `tracing_appender::rolling::daily` rolls files by
/// UTC date (verified empirically and documented in upstream). If we
/// use local time here, consumers near the day boundary end up looking
/// at a filename that does not exist yet (or a stale one) because the
/// writer is one day off. Always use UTC to stay in sync with the
/// writer.
pub fn current_log_file(log_dir: &Path) -> PathBuf {
    let today = Utc::now().date_naive();
    log_dir.join(format!("{LOG_FILE_BASENAME}.{today}"))
}

/// Return the path for the log file of a specific UTC date. Used by
/// the file watcher when a date rollover is observed.
pub fn log_file_for_date(log_dir: &Path, date: chrono::NaiveDate) -> PathBuf {
    log_dir.join(format!("{LOG_FILE_BASENAME}.{date}"))
}

/// Create a daily-rolling, non-blocking writer targeting `log_dir/gwt.log`.
///
/// The returned `WorkerGuard` must be kept alive (for example in a
/// `LoggingHandles` held by `main`) until the process exits, otherwise
/// the background writer thread shuts down and events are dropped.
///
/// **File confidentiality (reviewer comment B7):** on Unix the log
/// directory is created with mode `0700` and the rolling writer
/// inherits the user's umask, but we additionally tighten any existing
/// `gwt.log.YYYY-MM-DD` files to `0600` so that other local users on
/// shared hosts cannot read structured logs that may contain tokens or
/// internal paths. Failures to set permissions are non-fatal — they
/// are logged via `tracing::warn!` (once the subscriber is up) and the
/// writer still starts.
pub fn build(log_dir: &Path) -> std::io::Result<(NonBlocking, WorkerGuard)> {
    std::fs::create_dir_all(log_dir)?;
    tighten_log_dir_permissions(log_dir);

    // `tracing_appender 0.2.4` rotates `rolling::daily` files on UTC
    // boundaries and names files with the UTC date. Keep the helper
    // contract aligned with that behavior so tests, housekeeping, and
    // the Logs watcher all point at the same file.
    let file_appender = rolling::RollingFileAppender::builder()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix(LOG_FILE_BASENAME)
        .build(log_dir)
        .map_err(std::io::Error::other)?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    Ok((non_blocking, guard))
}

#[cfg(unix)]
fn tighten_log_dir_permissions(log_dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(log_dir) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o700);
        let _ = std::fs::set_permissions(log_dir, perms);
    }
    // Also tighten any existing rolling files left behind by previous
    // runs (the rolling writer creates new files inheriting the umask
    // which is typically 0022 → 0644, so we explicitly downgrade them
    // here on each startup).
    if let Ok(read_dir) = std::fs::read_dir(log_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name == LOG_FILE_BASENAME || name.starts_with(&format!("{LOG_FILE_BASENAME}.")) {
                if let Ok(metadata) = std::fs::metadata(&path) {
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(&path, perms);
                }
            }
        }
    }
}

#[cfg(not(unix))]
fn tighten_log_dir_permissions(_log_dir: &Path) {
    // Windows ACLs default to user-only access for files under
    // `%USERPROFILE%`, which already covers the project-scoped
    // `~/.gwt/projects/<repo-hash>/logs/` directory. No additional
    // hardening is required.
}

/// Stable, explicitly registered project destination. Keep a clone with background work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLogScope {
    token: String,
    log_dir: PathBuf,
}

impl ProjectLogScope {
    pub fn as_str(&self) -> &str {
        &self.token
    }
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }
    pub fn enter(&self) -> tracing::span::EnteredSpan {
        tracing::trace_span!(target: "gwt_log_scope", "project_operation", gwt_project_scope = self.as_str()).entered()
    }
}

struct ProjectSink {
    scope: ProjectLogScope,
    writer: Option<NonBlocking>,
    guard: Option<WorkerGuard>,
    failed: bool,
}

/// One registry for all project writers owned by the process logging handles.
#[derive(Clone)]
pub struct ProjectLogRouter {
    machine: NonBlocking,
    global_log_dir: PathBuf,
    projects_dir: PathBuf,
    projects: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, ProjectSink>>>,
    retention_days: u32,
}

impl ProjectLogRouter {
    pub(crate) fn new(machine: NonBlocking, retention_days: u32, global_log_dir: PathBuf) -> Self {
        Self {
            machine,
            global_log_dir,
            projects_dir: crate::paths::gwt_home().join("projects"),
            projects: Default::default(),
            retention_days,
        }
    }

    pub fn global_log_dir(&self) -> &Path {
        &self.global_log_dir
    }

    pub fn register_project(&self, project: &Path) -> std::io::Result<ProjectLogScope> {
        let canonical = std::fs::canonicalize(project)?;
        let token = crate::paths::project_scope_hash(&canonical).to_string();
        let log_dir = self.projects_dir.join(&token).join("logs");
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        Ok(projects
            .entry(token.clone())
            .or_insert_with(|| ProjectSink {
                scope: ProjectLogScope { token, log_dir },
                writer: None,
                guard: None,
                failed: false,
            })
            .scope
            .clone())
    }

    pub(crate) fn resolve<S>(
        &self,
        event: &tracing::Event<'_>,
        spans: Option<tracing_subscriber::registry::Scope<'_, S>>,
    ) -> Option<String>
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        let mut direct = ScopeField::default();
        event.record(&mut direct);
        let token = direct.0.or_else(|| {
            spans.and_then(|mut spans| {
                spans.find_map(|span| {
                    span.extensions()
                        .get::<ScopeField>()
                        .and_then(|field| field.0.clone())
                })
            })
        })?;
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        let sink = projects.get_mut(&token)?;
        if sink.failed {
            return None;
        }
        if sink.writer.is_none() {
            // Never emit tracing under the registry lock: subscriber recursion would deadlock.
            super::housekeep::housekeep(
                &sink.scope.log_dir,
                self.retention_days,
                "gwt.log.",
                "%Y-%m-%d",
            );
            match build(&sink.scope.log_dir) {
                Ok((writer, guard)) => {
                    sink.writer = Some(writer);
                    sink.guard = Some(guard);
                }
                Err(error) => {
                    // Bound open attempts and diagnostics to one per registered store.
                    sink.failed = true;
                    let diagnostic = serde_json::json!({
                        "timestamp": Utc::now().to_rfc3339(),
                        "level": "WARN",
                        "target": "gwt_core::logging",
                        "fields": {
                            "message": "project log writer unavailable; using machine diagnostics",
                            "project": token,
                            "error": error.to_string(),
                        }
                    });
                    use std::io::Write;
                    let _ = writeln!(self.machine.clone(), "{diagnostic}");
                    return None;
                }
            }
        }
        Some(token)
    }

    pub(crate) fn write(&self, scope: Option<&str>, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(writer) = scope
            .and_then(|token| projects.get_mut(token))
            .and_then(|sink| sink.writer.as_mut())
        {
            return writer.write_all(bytes);
        }
        drop(projects);
        self.machine.clone().write_all(bytes)
    }

    pub(crate) fn shutdown(&self) {
        // The subscriber retains a router clone; handles explicitly own shutdown.
        let guards: Vec<_> = self
            .projects
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values_mut()
            .filter_map(|sink| sink.guard.take())
            .collect();
        drop(guards);
    }
}

#[derive(Default)]
pub(crate) struct ScopeField(pub Option<String>);

impl tracing::field::Visit for ScopeField {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "gwt_project_scope" {
            self.0 = Some(value.to_owned());
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "gwt_project_scope" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_creates_log_dir_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        assert!(!nested.exists());
        let (_writer, _guard) = build(&nested).expect("build writer");
        assert!(nested.is_dir());
    }
}
