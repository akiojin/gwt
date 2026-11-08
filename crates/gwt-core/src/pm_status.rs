//! Durable PM-authored current status, independent of report history.

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
pub enum PmStatusItemKind {
    Working,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmStatusItem {
    pub kind: PmStatusItemKind,
    pub text: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmStatusInput {
    pub items: Vec<PmStatusItem>,
    pub next_trigger: String,
    #[serde(default)]
    pub user_action: Option<String>,
    #[serde(default = "default_stale_after_minutes")]
    pub stale_after_minutes: u32,
}

fn default_stale_after_minutes() -> u32 {
    15
}

impl PmStatusInput {
    pub fn validate(&self) -> Result<()> {
        validate_content(
            &self.items,
            &self.next_trigger,
            self.user_action.as_deref(),
            self.stale_after_minutes,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmCurrentStatus {
    pub items: Vec<PmStatusItem>,
    pub next_trigger: String,
    #[serde(default)]
    pub user_action: Option<String>,
    pub stale_after_minutes: u32,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub monitor_fingerprint: Option<String>,
}

impl PmCurrentStatus {
    /// Elapsed whole minutes once the configured stale threshold is reached.
    pub fn stale_minutes(&self, now: DateTime<Utc>) -> Option<i64> {
        let elapsed = now.signed_duration_since(self.updated_at).num_minutes();
        (elapsed >= i64::from(self.stale_after_minutes)).then_some(elapsed)
    }
}

fn validate_content(
    items: &[PmStatusItem],
    next_trigger: &str,
    user_action: Option<&str>,
    stale_after_minutes: u32,
) -> Result<()> {
    let message = if !(1..=3).contains(&items.len()) {
        Some("PM status must contain 1 to 3 items")
    } else if items.iter().any(|item| item.text.trim().is_empty()) {
        Some("PM status item text must be nonempty")
    } else if next_trigger.trim().is_empty() {
        Some("PM status next_trigger must be nonempty")
    } else if user_action.is_some_and(|action| action.trim().is_empty()) {
        Some("PM status user_action must be nonempty when provided")
    } else if stale_after_minutes == 0 {
        Some("PM status stale_after_minutes must be positive")
    } else {
        None
    };
    match message {
        Some(message) => Err(GwtError::Config(message.into())),
        None => Ok(()),
    }
}

/// Current explanations share the repository's machine-local project store.
pub fn status_path(repo_path: &Path) -> PathBuf {
    crate::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/pm-status.jsonl")
}

/// A successful change has been synced and read back under the exclusive lock.
/// Identical content and Monitor state return the prior event unchanged.
pub fn set_status(
    repo_path: &Path,
    input: PmStatusInput,
    monitor_fingerprint: Option<String>,
) -> Result<PmCurrentStatus> {
    set_status_from_path(&status_path(repo_path), input, monitor_fingerprint)
}

/// Write to the same resolved store used by PM current-state readers.
pub fn set_status_from_path(
    path: &Path,
    input: PmStatusInput,
    monitor_fingerprint: Option<String>,
) -> Result<PmCurrentStatus> {
    input.validate()?;
    fs::create_dir_all(path.parent().expect("status store has a parent"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    file.lock_exclusive()?;
    let history = read_status_history(&mut file, path)?;
    if let Some(previous) = history.last() {
        if previous.items == input.items
            && previous.next_trigger == input.next_trigger
            && previous.user_action == input.user_action
            && previous.stale_after_minutes == input.stale_after_minutes
            && previous.monitor_fingerprint == monitor_fingerprint
        {
            return Ok(previous.clone());
        }
    }
    let status = PmCurrentStatus {
        items: input.items,
        next_trigger: input.next_trigger,
        user_action: input.user_action,
        stale_after_minutes: input.stale_after_minutes,
        updated_at: Utc::now(),
        monitor_fingerprint,
    };
    append_status(&mut file, path, &status, |file, encoded| {
        file.write_all(encoded)
    })?;
    Ok(status)
}

pub fn load_status_history(repo_path: &Path) -> Result<Vec<PmCurrentStatus>> {
    load_status_history_from_path(&status_path(repo_path))
}

pub fn latest_status(repo_path: &Path) -> Result<Option<PmCurrentStatus>> {
    latest_status_from_path(&status_path(repo_path))
}

/// Read a store already resolved by the PM-worktree boundary.
pub fn latest_status_from_path(status_file: &Path) -> Result<Option<PmCurrentStatus>> {
    Ok(load_status_history_from_path(status_file)?.pop())
}

/// Read the complete history under the shared lock at an already resolved path.
pub fn load_status_history_from_path(path: &Path) -> Result<Vec<PmCurrentStatus>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    FileExt::lock_shared(&file)?;
    read_status_history(&mut file, path)
}

fn append_status(
    file: &mut File,
    path: &Path,
    status: &PmCurrentStatus,
    append: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<()> {
    let offset = file.seek(SeekFrom::End(0))?;
    let result = (|| -> Result<()> {
        let mut encoded = serde_json::to_vec(status).map_err(invalid_status_data)?;
        encoded.push(b'\n');
        append(file, &encoded)?;
        file.sync_all()?;
        sync_status_parent(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut persisted = String::new();
        file.read_to_string(&mut persisted)?;
        let readback: PmCurrentStatus =
            serde_json::from_str(&persisted).map_err(invalid_status_data)?;
        if &readback != status {
            return Err(invalid_status_data(
                "PM status readback did not match the submitted status",
            )
            .into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        // The exclusive lock prevents another append before rollback.
        if let Err(rollback) = file.set_len(offset).and_then(|()| file.sync_all()) {
            return Err(io::Error::other(format!(
                "PM status append failed: {error}; rollback to byte {offset} failed: {rollback}"
            ))
            .into());
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn sync_status_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().expect("status store has a parent"))?.sync_all()
}

#[cfg(not(unix))]
fn sync_status_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn read_status_history(file: &mut File, path: &Path) -> Result<Vec<PmCurrentStatus>> {
    file.seek(SeekFrom::Start(0))?;
    let mut history = Vec::new();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_number = 0;
    while reader.read_line(&mut line)? != 0 {
        line_number += 1;
        if !line.ends_with('\n') {
            return Err(invalid_status_data(format!(
                "{} line {line_number}: incomplete PM status record (missing newline)",
                path.display()
            ))
            .into());
        }
        if !line.trim().is_empty() {
            let status: PmCurrentStatus = serde_json::from_str(&line).map_err(|error| {
                invalid_status_data(format!("{} line {line_number}: {error}", path.display()))
            })?;
            validate_content(
                &status.items,
                &status.next_trigger,
                status.user_action.as_deref(),
                status.stale_after_minutes,
            )
            .map_err(|error| {
                invalid_status_data(format!("{} line {line_number}: {error}", path.display()))
            })?;
            history.push(status);
        }
        line.clear();
    }
    Ok(history)
}

fn invalid_status_data(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;
    use chrono::Duration;

    fn input() -> PmStatusInput {
        PmStatusInput {
            items: vec![PmStatusItem {
                kind: PmStatusItemKind::Working,
                text: "Reviewing the release queue".into(),
                started_at: "2026-10-13T00:00:00Z".parse().unwrap(),
            }],
            next_trigger: "Required CI finishes".into(),
            user_action: None,
            stale_after_minutes: 15,
        }
    }

    #[test]
    fn status_history_survives_reload_without_touching_report_history() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        assert!(load_status_history(&repo).unwrap().is_empty());
        assert!(latest_status(&repo).unwrap().is_none());
        let first = set_status(&repo, input(), None).unwrap();
        let mut next = input();
        next.items[0].kind = PmStatusItemKind::Waiting;
        next.user_action = Some("Confirm the release target".into());
        let second = set_status(&repo, next, None).unwrap();
        assert!(status_path(&repo).starts_with(home.path().join(".gwt/projects")));
        assert!(status_path(&repo).ends_with("project-state/pm-status.jsonl"));
        assert_eq!(load_status_history(&repo).unwrap(), [first, second.clone()]);
        assert_eq!(
            latest_status_from_path(&status_path(&repo)).unwrap(),
            Some(second.clone())
        );
        assert_eq!(latest_status(&repo).unwrap(), Some(second));
        assert!(!crate::pm_report::reports_path(&repo).exists());
    }

    #[test]
    fn identical_cycles_do_not_renew_status_but_changed_monitor_state_is_acknowledged() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let first = set_status(&repo, input(), Some("cycle-one".into())).unwrap();
        let bytes = std::fs::read(status_path(&repo)).unwrap();
        let duplicate = set_status(&repo, input(), Some("cycle-one".into())).unwrap();
        assert_eq!(first, duplicate);
        assert_eq!(std::fs::read(status_path(&repo)).unwrap(), bytes);
        let acknowledged = set_status(&repo, input(), Some("cycle-two".into())).unwrap();
        assert_eq!(
            acknowledged.monitor_fingerprint.as_deref(),
            Some("cycle-two")
        );
        assert_eq!(load_status_history(&repo).unwrap(), [first, acknowledged]);
    }

    #[test]
    fn status_validation_rejects_invalid_card_content() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let base = input();
        let mut empty = base.clone();
        empty.items.clear();
        let mut too_many = base.clone();
        too_many.items = vec![base.items[0].clone(); 4];
        let mut blank_item = base.clone();
        blank_item.items[0].text = " \n ".into();
        let mut blank_trigger = base.clone();
        blank_trigger.next_trigger = " ".into();
        let mut blank_action = base.clone();
        blank_action.user_action = Some(" ".into());
        let mut zero_threshold = base;
        zero_threshold.stale_after_minutes = 0;
        for invalid in [
            empty,
            too_many,
            blank_item,
            blank_trigger,
            blank_action,
            zero_threshold,
        ] {
            assert!(set_status(&repo, invalid, None).is_err());
        }
        assert!(!status_path(&repo).exists());
        let defaults: PmStatusInput = serde_json::from_value(serde_json::json!({
            "items": [{"kind":"waiting","text":"Awaiting CI","started_at":"2026-10-13T00:00:00Z"}],
            "next_trigger":"CI completes"
        }))
        .unwrap();
        assert_eq!(defaults.stale_after_minutes, 15);
        assert!(defaults.user_action.is_none());
        assert!(serde_json::from_value::<PmStatusItem>(serde_json::json!({
            "kind":"complete","text":"Finished","started_at":"2026-10-13T00:00:00Z"
        }))
        .is_err());
    }

    #[test]
    fn staleness_uses_the_last_real_update_and_a_deterministic_clock() {
        let updated_at: DateTime<Utc> = "2026-10-13T00:00:00Z".parse().unwrap();
        let status = PmCurrentStatus {
            items: input().items,
            next_trigger: "CI completes".into(),
            user_action: None,
            stale_after_minutes: 15,
            updated_at,
            monitor_fingerprint: None,
        };
        assert_eq!(
            status.stale_minutes(updated_at - Duration::minutes(1)),
            None
        );
        assert_eq!(
            status.stale_minutes(updated_at + Duration::seconds(899)),
            None
        );
        assert_eq!(
            status.stale_minutes(updated_at + Duration::minutes(15)),
            Some(15)
        );
        assert_eq!(
            status.stale_minutes(updated_at + Duration::minutes(23)),
            Some(23)
        );
    }

    #[test]
    fn damaged_or_unwritable_status_history_is_reported_without_appending() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let path = status_path(&repo);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(set_status(&repo, input(), None).is_err());
        assert!(load_status_history(&repo).is_err());
        std::fs::remove_dir(&path).unwrap();
        set_status(&repo, input(), None).unwrap();
        let mut damaged = std::fs::read_to_string(&path).unwrap();
        damaged.pop();
        std::fs::write(&path, &damaged).unwrap();
        assert!(load_status_history(&repo).is_err());
        assert!(set_status(&repo, input(), Some("changed".into())).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), damaged);
    }

    #[test]
    fn failed_partial_append_restores_the_previous_status_history() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let previous = set_status(&repo, input(), None).unwrap();
        let path = status_path(&repo);
        let original = fs::read(&path).unwrap();
        let failed = PmCurrentStatus {
            next_trigger: "Must not persist".into(),
            ..previous.clone()
        };
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.lock_exclusive().unwrap();
        let error = append_status(&mut file, &path, &failed, |file, encoded| {
            file.write_all(&encoded[..7])?;
            Err(io::Error::other("injected partial write failure"))
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("injected partial write failure"),
            "{error}"
        );
        drop(file);
        assert_eq!(load_status_history(&repo).unwrap(), [previous]);
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
