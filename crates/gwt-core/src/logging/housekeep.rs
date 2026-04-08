//! Startup housekeeping: delete rotated log files older than the retention window.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};

use super::writer::LOG_FILE_BASENAME;

/// Summary of a housekeeping run. Non-fatal errors are collected into
/// `errors` rather than being returned as `Err`, so that a single
/// unreadable file cannot block TUI startup.
#[derive(Debug, Default)]
pub struct HousekeepReport {
    pub inspected: usize,
    pub deleted: Vec<PathBuf>,
    pub errors: Vec<(PathBuf, String)>,
}

/// Delete rotated log files older than `retention_days` relative to today's
/// UTC date. Returns a `HousekeepReport` describing what was done.
///
/// `retention_days == 0` disables housekeeping entirely. The active file
/// (`gwt.log`) and files whose date suffix cannot be parsed are left
/// untouched.
pub fn housekeep(log_dir: &Path, retention_days: u32) -> HousekeepReport {
    housekeep_at(log_dir, retention_days, Utc::now().date_naive())
}

/// Deterministic version of `housekeep` that lets tests pin `today`.
pub fn housekeep_at(log_dir: &Path, retention_days: u32, today: NaiveDate) -> HousekeepReport {
    let mut report = HousekeepReport::default();
    if retention_days == 0 {
        return report;
    }

    let entries = match fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(err) => {
            // Missing directory is not an error — nothing to clean up.
            if err.kind() != std::io::ErrorKind::NotFound {
                report.errors.push((log_dir.to_path_buf(), err.to_string()));
            }
            return report;
        }
    };

    // "Keep the last N days (inclusive of today)" ⇒ cutoff = today - (N - 1).
    // A file dated exactly `cutoff` is still within the retention window.
    let cutoff = today - chrono::Duration::days((retention_days.saturating_sub(1)) as i64);

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(name) => name,
            None => continue,
        };
        report.inspected += 1;

        // Active file: skip.
        if file_name == LOG_FILE_BASENAME {
            continue;
        }

        // Rotated files look like `gwt.log.YYYY-MM-DD`.
        let Some(suffix) = file_name.strip_prefix(&format!("{LOG_FILE_BASENAME}.")) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(suffix, "%Y-%m-%d") else {
            continue;
        };

        if date < cutoff {
            match fs::remove_file(&path) {
                Ok(()) => report.deleted.push(path),
                Err(err) => report.errors.push((path, err.to_string())),
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, b"").expect("write file");
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");
        let report = housekeep(&missing, 7);
        assert!(report.errors.is_empty());
        assert_eq!(report.inspected, 0);
    }

    #[test]
    fn retention_zero_disables_cleanup() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("gwt.log.2020-01-01"));
        let report = housekeep(dir.path(), 0);
        assert!(report.deleted.is_empty());
        assert!(dir.path().join("gwt.log.2020-01-01").exists());
    }

    #[test]
    fn deletes_only_files_older_than_retention() {
        let dir = tempfile::tempdir().expect("tempdir");
        let today = NaiveDate::from_ymd_opt(2026, 4, 10).unwrap();
        touch(&dir.path().join("gwt.log")); // active — must survive
        touch(&dir.path().join("gwt.log.2026-04-09")); // 1 day old
        touch(&dir.path().join("gwt.log.2026-04-04")); // 6 days old (boundary)
        touch(&dir.path().join("gwt.log.2026-04-03")); // 7 days old → deleted
        touch(&dir.path().join("gwt.log.2026-03-15")); // way old → deleted
        touch(&dir.path().join("unrelated.txt")); // unrelated — ignored

        let report = housekeep_at(dir.path(), 7, today);

        assert_eq!(report.deleted.len(), 2);
        assert!(dir.path().join("gwt.log").exists());
        assert!(dir.path().join("gwt.log.2026-04-09").exists());
        assert!(dir.path().join("gwt.log.2026-04-04").exists());
        assert!(!dir.path().join("gwt.log.2026-04-03").exists());
        assert!(!dir.path().join("gwt.log.2026-03-15").exists());
        assert!(dir.path().join("unrelated.txt").exists());
    }

    #[test]
    fn malformed_suffix_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("gwt.log.not-a-date"));
        let today = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();
        let report = housekeep_at(dir.path(), 7, today);
        assert!(report.deleted.is_empty());
        assert!(dir.path().join("gwt.log.not-a-date").exists());
    }
}
