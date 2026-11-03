//! Non-blocking, size-bounded daily JSONL writer.
//!
//! **File naming:** no bare active file is maintained. Every event is written to
//! `{log_dir}/gwt.log.YYYY-MM-DD` where the date is the current UTC
//! day. The Logs tab and housekeeping code must therefore reference the
//! dated filename directly. Use `current_log_file()` to compute it.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use chrono::Utc;
use fs2::FileExt;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};

/// Basename prefix used for the rolling log files. The full name is
/// `gwt.log.YYYY-MM-DD` (no bare `gwt.log` file exists at any point).
pub const LOG_FILE_BASENAME: &str = "gwt.log";

/// Hard ceiling for canonical log files in each machine/project log directory.
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Return the path of today's active log file
/// (`{log_dir}/gwt.log.YYYY-MM-DD`, UTC date).
///
/// The writer and consumers both use UTC so they always choose the same
/// daily filename, including near a local midnight boundary.
pub fn current_log_file(log_dir: &Path) -> PathBuf {
    let today = Utc::now().date_naive();
    log_dir.join(format!("{LOG_FILE_BASENAME}.{today}"))
}

/// Return the path for the log file of a specific UTC date. Used by
/// the file watcher when a date rollover is observed.
pub fn log_file_for_date(log_dir: &Path, date: chrono::NaiveDate) -> PathBuf {
    log_dir.join(format!("{LOG_FILE_BASENAME}.{date}"))
}

/// Create a daily, non-blocking writer with a 2 GiB ceiling per log directory.
/// Old daily files are deleted first; reaching the ceiling within the active
/// day resets that file before writing the next complete record.
///
/// The returned `WorkerGuard` must be kept alive (for example in a
/// `LoggingHandles` held by `main`) until the process exits, otherwise
/// the background writer thread shuts down and events are dropped.
///
/// **File confidentiality (reviewer comment B7):** on Unix the log
/// directory is tightened to mode `0700` and new files use `0600`.
/// We additionally tighten any existing `gwt.log.YYYY-MM-DD` files so that other local users on
/// shared hosts cannot read structured logs that may contain tokens or
/// internal paths. Failures to tighten existing permissions are non-fatal.
pub fn build(log_dir: &Path) -> std::io::Result<(NonBlocking, WorkerGuard)> {
    std::fs::create_dir_all(log_dir)?;
    tighten_log_dir_permissions(log_dir);

    let file_appender = BoundedDailyWriter::new(log_dir, MAX_LOG_BYTES)?;
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    Ok((non_blocking, guard))
}

struct BoundedDailyWriter {
    log_dir: PathBuf,
    lock: File,
    max_bytes: u64,
    partial_record: Vec<u8>,
    discarding_record: bool,
}

impl BoundedDailyWriter {
    fn new(log_dir: &Path, max_bytes: u64) -> io::Result<Self> {
        std::fs::create_dir_all(log_dir)?;
        let writer = Self {
            log_dir: log_dir.to_path_buf(),
            lock: open_log_file(&log_dir.join(".gwt-log-size.lock"))?,
            max_bytes,
            partial_record: Vec::new(),
            discarding_record: false,
        };
        // Apply the ceiling to logs left by previous versions before accepting
        // any new events, including an already oversized active daily file.
        writer.write_record(&[])?;
        Ok(writer)
    }

    fn write_record(&self, record: &[u8]) -> io::Result<()> {
        // Do not use NamedFileLock or tracing here: logging about this lock
        // would recursively enter the subscriber. All cooperating GUI/gwtd
        // writers use the same stable lock and reopen the current file per
        // record, so a peer's truncation never leaves a stale file handle.
        self.lock.lock_exclusive()?;
        let result = (|| {
            let path = current_log_file(&self.log_dir);
            let mut file = open_log_file(&path)?;
            super::housekeep::enforce_size_limit(
                &self.log_dir,
                &path,
                self.max_bytes,
                record.len() as u64,
            )?;
            file.write_all(record)
        })();
        let unlock = FileExt::unlock(&self.lock);
        result.and_then(|()| unlock)
    }
}

impl Write for BoundedDailyWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // RoutedJson normally submits one complete JSONL record. Buffer other
        // callers' formatting chunks until a newline so size rotation cannot
        // expose a partial JSON record or interleave it with another writer.
        for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
            let complete = chunk.ends_with(b"\n");
            if self.discarding_record {
                self.discarding_record = !complete;
                continue;
            }
            if (self.partial_record.len() as u64).saturating_add(chunk.len() as u64)
                > self.max_bytes
            {
                self.partial_record.clear();
                self.discarding_record = !complete;
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "log record exceeds the log directory size limit",
                ));
            }
            if self.partial_record.is_empty() && complete {
                self.write_record(chunk)?;
            } else {
                self.partial_record.extend_from_slice(chunk);
                if complete {
                    let record = std::mem::take(&mut self.partial_record);
                    self.write_record(&record)?;
                }
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Completed records are unbuffered; never persist an incomplete tail.
        Ok(())
    }
}

fn open_log_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    // LockFileEx on Windows rejects an append-only handle; the stable lock
    // file needs read access as well. The log writer still always appends.
    options.create(true).read(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let file = options.open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log destination is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn tighten_log_dir_permissions(log_dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(log_dir) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o700);
        let _ = std::fs::set_permissions(log_dir, perms);
    }
    // Also tighten files left by older versions whose rolling writer
    // inherited a permissive umask.
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
        // Machine events must not wait for project writer creation/housekeeping.
        if scope.is_none() {
            return self.machine.clone().write_all(bytes);
        }
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
    use std::io::Write;

    use super::*;

    #[test]
    fn bounded_writers_share_the_limit_and_keep_complete_records() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = BoundedDailyWriter::new(dir.path(), 32).unwrap();
        let mut second = BoundedDailyWriter::new(dir.path(), 32).unwrap();
        for value in 0..6 {
            let writer = if value % 2 == 0 {
                &mut first
            } else {
                &mut second
            };
            writer
                .write_all(format!("{{\"n\":{value}}}\n").as_bytes())
                .unwrap();
        }

        let path = current_log_file(dir.path());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"n\":4}\n{\"n\":5}\n"
        );
        first.write_all(b"{\"n\":").unwrap();
        second.write_all(b"{\"n\":7}\n").unwrap();
        first.write_all(b"6}\n").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "{\"n\":4}\n{\"n\":5}\n{\"n\":7}\n{\"n\":6}\n");
        assert!(std::fs::metadata(&path).unwrap().len() <= 32);
        for line in contents.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
        assert!(first
            .write_all(b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n")
            .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
        first.write_all(b"{\"n\":8}\n").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{\"n\":8}\n");

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers = [first, second]
            .into_iter()
            .enumerate()
            .map(|(id, mut writer)| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    for value in 0..50 {
                        barrier.wait();
                        writer
                            .write_all(format!("{{\"writer\":{id},\"n\":{value}}}\n").as_bytes())
                            .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        let contents = std::fs::read_to_string(current_log_file(dir.path())).unwrap();
        assert!(contents.len() <= 32);
        assert!(!contents.is_empty());
        for line in contents.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["n"], 49);
        }
    }

    #[test]
    fn build_creates_log_dir_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        assert!(!nested.exists());
        let (_writer, _guard) = build(&nested).expect("build writer");
        assert!(nested.is_dir());
    }

    #[test]
    fn machine_log_does_not_wait_for_project_writer_registry() {
        let dir = tempfile::tempdir().unwrap();
        let (machine, guard) = build(dir.path()).unwrap();
        let router = ProjectLogRouter::new(machine, 7, dir.path().to_path_buf());
        let registry = router.projects.lock().unwrap();
        let worker_router = router.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            tx.send(worker_router.write(None, b"machine marker\n"))
                .unwrap();
        });
        // test-hygiene: allow-wall-clock-deadline hang guard; completion must precede releasing the unrelated registry lock
        let result = rx.recv_timeout(std::time::Duration::from_secs(30));
        // Release even on failure so the blocked worker can join before assert.
        drop(registry);
        worker.join().unwrap();
        result
            .expect("machine log waited for the project writer registry")
            .unwrap();
        drop(router);
        drop(guard);
        assert_eq!(
            std::fs::read_to_string(current_log_file(dir.path())).unwrap(),
            "machine marker\n"
        );
    }
}
