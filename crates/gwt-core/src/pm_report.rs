//! Durable PM-authored reports, separate from terminal and conversation history.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::{GwtError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmReportKind {
    Progress,
    Decision,
    Blocker,
    Qa,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmReport {
    pub id: String,
    pub kind: PmReportKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

/// Reports share the repository's machine-local project store and never expire.
pub fn reports_path(repo_path: &Path) -> PathBuf {
    crate::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/pm-reports.jsonl")
}

/// A successful post has been appended, synced and read back while holding the lock.
pub fn post_report(
    repo_path: &Path,
    kind: PmReportKind,
    body: &str,
    question: Option<&str>,
) -> Result<PmReport> {
    if kind == PmReportKind::Qa && question.is_none_or(|value| value.trim().is_empty()) {
        return Err(GwtError::Config(
            "PM Q&A report question must be nonempty".into(),
        ));
    }
    if question.is_some_and(|value| value.len() > 64 * 1024) {
        return Err(GwtError::Config(
            "PM report question must not exceed 64 KiB".into(),
        ));
    }
    if body.trim().is_empty() {
        return Err(GwtError::Config("PM report body must be nonempty".into()));
    }
    if body.len() > 64 * 1024 {
        return Err(GwtError::Config(
            "PM report body must not exceed 64 KiB".into(),
        ));
    }
    let path = reports_path(repo_path);
    fs::create_dir_all(path.parent().expect("report store has a parent"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    file.lock_exclusive()?;
    // Reject damaged history before appending a success the reader cannot display.
    read_reports(&mut file, &path)?;
    let report = PmReport {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        question: question.map(str::to_owned),
        body: body.to_owned(),
        created_at: Utc::now(),
    };
    append_report(&mut file, &path, &report, |file, encoded| {
        file.write_all(encoded)
    })?;
    Ok(report)
}

fn append_report(
    file: &mut File,
    path: &Path,
    report: &PmReport,
    append: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<()> {
    let offset = file.seek(SeekFrom::End(0))?;
    let result = (|| -> Result<()> {
        let mut encoded = serde_json::to_vec(report).map_err(invalid_report_data)?;
        encoded.push(b'\n');
        append(file, &encoded)?;
        file.sync_all()?;
        sync_report_parent(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut persisted = String::new();
        file.read_to_string(&mut persisted)?;
        let readback: PmReport = serde_json::from_str(&persisted).map_err(invalid_report_data)?;
        if &readback != report {
            return Err(invalid_report_data(
                "PM report readback did not match the submitted report",
            )
            .into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        // The caller still holds the exclusive lock, so no other writer can
        // append between this transaction and restoring the previous length.
        if let Err(rollback) = file.set_len(offset).and_then(|()| file.sync_all()) {
            return Err(io::Error::other(format!(
                "PM report append failed: {error}; rollback to byte {offset} failed: {rollback}"
            ))
            .into());
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn sync_report_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().expect("report store has a parent"))?.sync_all()
}

#[cfg(not(unix))]
fn sync_report_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn load_reports(repo_path: &Path) -> Result<Vec<PmReport>> {
    let path = reports_path(repo_path);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    FileExt::lock_shared(&file)?;
    read_reports(&mut file, &path)
}

fn read_reports(file: &mut File, path: &Path) -> Result<Vec<PmReport>> {
    let mut reports = Vec::new();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_number = 0;
    while reader.read_line(&mut line)? != 0 {
        line_number += 1;
        if !line.ends_with('\n') {
            return Err(invalid_report_data(format!(
                "{} line {line_number}: incomplete PM report record (missing newline)",
                path.display()
            ))
            .into());
        }
        if !line.trim().is_empty() {
            reports.push(serde_json::from_str(&line).map_err(|error| {
                invalid_report_data(format!("{} line {line_number}: {error}", path.display()))
            })?);
        }
        line.clear();
    }
    Ok(reports)
}

fn invalid_report_data(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;

    #[test]
    fn reports_survive_reload_in_project_order() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        assert!(load_reports(&repo).unwrap().is_empty());
        let first =
            post_report(&repo, PmReportKind::Progress, "# Progress\n\n- Done", None).unwrap();
        let second = post_report(
            &repo,
            PmReportKind::Decision,
            "Keep the existing terminal",
            None,
        )
        .unwrap();
        assert_ne!(first.id, second.id);
        assert!(reports_path(&repo).starts_with(home.path().join(".gwt/projects")));
        assert!(reports_path(&repo).ends_with("project-state/pm-reports.jsonl"));
        assert_eq!(load_reports(&repo).unwrap(), [first, second]);
        assert_eq!(
            std::fs::read_to_string(reports_path(&repo))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[test]
    fn qa_reports_preserve_question_with_legacy_records() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let path = reports_path(&repo);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let legacy = ["progress", "decision", "blocker"]
            .map(|kind| {
                serde_json::json!({
                    "id": kind, "kind": kind, "body": "Existing report",
                    "created_at": "2026-10-13T00:00:00Z"
                })
                .to_string()
            })
            .join("\n");
        fs::write(&path, format!("{legacy}\n")).unwrap();
        let qa = post_report(
            &repo,
            PmReportKind::Qa,
            "Yes. Verified the checks.",
            Some("Can we ship?"),
        )
        .unwrap();

        let reports = load_reports(&repo).unwrap();
        assert_eq!(reports.len(), 4);
        for report in &reports[..3] {
            assert!(serde_json::to_value(report)
                .unwrap()
                .get("question")
                .is_none());
        }
        assert_eq!(reports[3], qa);
        assert_eq!(
            serde_json::to_value(&reports[3]).unwrap()["question"],
            "Can we ship?"
        );
    }

    #[test]
    fn report_validation_and_storage_failures_are_explicit() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        for question in [None, Some(" \n ")] {
            let error = post_report(&repo, PmReportKind::Qa, "Answer", question).unwrap_err();
            assert!(
                error.to_string().contains("question must be nonempty"),
                "{error}"
            );
        }
        let blank = post_report(&repo, PmReportKind::Progress, " \n ", None).unwrap_err();
        assert!(blank.to_string().contains("nonempty"), "{blank}");
        let oversized =
            post_report(&repo, PmReportKind::Progress, &"x".repeat(65_537), None).unwrap_err();
        assert!(oversized.to_string().contains("64 KiB"), "{oversized}");
        let oversized_question = "あ".repeat(21_846);
        for kind in [PmReportKind::Qa, PmReportKind::Progress] {
            let error = post_report(&repo, kind, "Answer", Some(&oversized_question)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("question must not exceed 64 KiB"),
                "{error}"
            );
        }
        let path = reports_path(&repo);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(post_report(&repo, PmReportKind::Blocker, "Cannot write", None).is_err());
        assert!(load_reports(&repo).is_err());
        std::fs::remove_dir(&path).unwrap();
        post_report(&repo, PmReportKind::Progress, "Complete record", None).unwrap();
        let mut truncated = std::fs::read_to_string(&path).unwrap();
        truncated.pop();
        std::fs::write(&path, &truncated).unwrap();
        assert!(
            load_reports(&repo).is_err(),
            "an incomplete final record must be reported"
        );
        assert!(post_report(&repo, PmReportKind::Blocker, "Must not append", None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), truncated);
    }

    #[test]
    fn failed_partial_append_preserves_previously_saved_reports() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let previous =
            post_report(&repo, PmReportKind::Progress, "Previously saved", None).unwrap();
        let path = reports_path(&repo);
        let original = fs::read(&path).unwrap();
        let report = PmReport {
            id: "failed".into(),
            body: "Not committed".into(),
            ..previous.clone()
        };
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.lock_exclusive().unwrap();
        let error = append_report(&mut file, &path, &report, |file, encoded| {
            file.write_all(&encoded[..7])?;
            Err(io::Error::other("injected partial write failure"))
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("injected partial write failure"),
            "{error}"
        );
        drop(file);
        assert_eq!(load_reports(&repo).unwrap(), [previous]);
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
