//! Startup housekeeping: delete rotated log files older than the retention window.

use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::{NaiveDate, Utc};

/// Summary of a housekeeping run. Non-fatal errors are collected into
/// `errors` rather than being returned as `Err`, so that a single
/// unreadable file cannot block TUI startup.
#[derive(Debug, Default)]
pub struct HousekeepReport {
    pub inspected: usize,
    pub deleted: Vec<PathBuf>,
    pub errors: Vec<(PathBuf, String)>,
}

/// Delete dated files older than `retention_days` relative to today's UTC
/// date. Matching files start with `file_name_prefix`; the remainder of the
/// file name must parse using `date_suffix_format`. Returns a
/// `HousekeepReport` describing what was done.
///
/// `retention_days == 0` disables housekeeping entirely. Files with another
/// prefix or an unparseable date suffix are left untouched.
pub fn housekeep(
    log_dir: &Path,
    retention_days: u32,
    file_name_prefix: &str,
    date_suffix_format: &str,
) -> HousekeepReport {
    housekeep_at(
        log_dir,
        retention_days,
        file_name_prefix,
        date_suffix_format,
        Utc::now().date_naive(),
    )
}

/// Deterministic version of `housekeep` that lets tests pin `today`.
pub fn housekeep_at(
    log_dir: &Path,
    retention_days: u32,
    file_name_prefix: &str,
    date_suffix_format: &str,
    today: NaiveDate,
) -> HousekeepReport {
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
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        report.inspected += 1;

        let Some(suffix) = file_name.strip_prefix(file_name_prefix) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(suffix, date_suffix_format) else {
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

/// Make room for a complete record under the writer's cross-process lock.
/// Only canonical daily log files participate; directories, symlinks and
/// unrelated diagnostics are never removed. Delete oldest history first,
/// then reset the active file if that alone cannot fit the next record.
pub(crate) fn enforce_size_limit(
    log_dir: &Path,
    active: &Path,
    max_bytes: u64,
    incoming_bytes: u64,
) -> std::io::Result<()> {
    let available = max_bytes.checked_sub(incoming_bytes).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "log record exceeds the log directory size limit",
        )
    })?;
    let mut files = Vec::new();
    let mut total = 0_u64;
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(suffix) = name.strip_prefix("gwt.log.") else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(suffix, "%Y-%m-%d") else {
            continue;
        };
        if suffix != date.to_string() {
            continue;
        }
        let bytes = entry.metadata()?.len();
        total = total.saturating_add(bytes);
        files.push((date, entry.path(), bytes));
    }
    files.sort_unstable_by_key(|(date, _, _)| *date);
    for (_, path, bytes) in files {
        if total <= available {
            break;
        }
        if path != active {
            fs::remove_file(path)?;
            total = total.saturating_sub(bytes);
        }
    }
    if total > available {
        fs::OpenOptions::new()
            .write(true)
            .open(active)?
            .set_len(0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GWT_LOG_PREFIX: &str = "gwt.log.";
    const GWT_LOG_DATE_SUFFIX_FORMAT: &str = "%Y-%m-%d";

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
        let report = housekeep(&missing, 7, GWT_LOG_PREFIX, GWT_LOG_DATE_SUFFIX_FORMAT);
        assert!(report.errors.is_empty());
        assert_eq!(report.inspected, 0);
    }

    #[test]
    fn retention_zero_disables_cleanup() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("gwt.log.2020-01-01"));
        let report = housekeep(dir.path(), 0, GWT_LOG_PREFIX, GWT_LOG_DATE_SUFFIX_FORMAT);
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

        let report = housekeep_at(
            dir.path(),
            7,
            GWT_LOG_PREFIX,
            GWT_LOG_DATE_SUFFIX_FORMAT,
            today,
        );

        assert_eq!(report.deleted.len(), 2);
        assert!(dir.path().join("gwt.log").exists());
        assert!(dir.path().join("gwt.log.2026-04-09").exists());
        assert!(dir.path().join("gwt.log.2026-04-04").exists());
        assert!(!dir.path().join("gwt.log.2026-04-03").exists());
        assert!(!dir.path().join("gwt.log.2026-03-15").exists());
        assert!(dir.path().join("unrelated.txt").exists());
    }

    #[test]
    fn deletes_daily_files_for_arbitrary_prefix_and_date_format() {
        let dir = tempfile::tempdir().expect("tempdir");
        let today = NaiveDate::from_ymd_opt(2026, 4, 10).unwrap();
        touch(&dir.path().join("perf-2026-04-04.jsonl")); // retention boundary
        touch(&dir.path().join("perf-2026-04-03.jsonl")); // expired
        touch(&dir.path().join("other-2026-04-03.jsonl")); // different prefix

        let report = housekeep_at(dir.path(), 7, "perf-", "%Y-%m-%d.jsonl", today);

        assert_eq!(
            report.deleted,
            vec![dir.path().join("perf-2026-04-03.jsonl")]
        );
        assert!(dir.path().join("perf-2026-04-04.jsonl").exists());
        assert!(!dir.path().join("perf-2026-04-03.jsonl").exists());
        assert!(dir.path().join("other-2026-04-03.jsonl").exists());
    }

    #[test]
    fn malformed_suffix_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("gwt.log.not-a-date"));
        let today = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();
        let report = housekeep_at(
            dir.path(),
            7,
            GWT_LOG_PREFIX,
            GWT_LOG_DATE_SUFFIX_FORMAT,
            today,
        );
        assert!(report.deleted.is_empty());
        assert!(dir.path().join("gwt.log.not-a-date").exists());
    }

    #[test]
    fn size_limit_prunes_oldest_logs_and_bounds_the_active_file() {
        let dir = tempfile::tempdir().unwrap();
        let oldest = dir.path().join("gwt.log.2026-04-08");
        let recent = dir.path().join("gwt.log.2026-04-09");
        let active = dir.path().join("gwt.log.2026-04-10");
        let unrelated = dir.path().join("gwt.log.not-a-date");
        for path in [&oldest, &recent, &active] {
            fs::write(path, b"12345678").unwrap();
        }
        fs::write(&unrelated, b"unrelated diagnostics").unwrap();

        enforce_size_limit(dir.path(), &active, 16, 0).unwrap();
        assert!(!oldest.exists());
        assert_eq!(fs::read(&recent).unwrap(), b"12345678");
        assert_eq!(fs::read(&active).unwrap(), b"12345678");

        enforce_size_limit(dir.path(), &active, 16, 12).unwrap();
        assert!(!recent.exists());
        assert!(fs::read(&active).unwrap().is_empty());
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated diagnostics");
    }
}
