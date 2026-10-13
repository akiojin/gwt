//! Issue #4680 — the PM's `gwt-spec` survey record.
//!
//! The registration gate has to answer one question: has this PM established
//! which `gwt-spec` owns the area before it registers a new Issue? The hook
//! process that observes `issue.spec.list` and the hook process that later
//! sees `issue.create` are different processes, so the answer cannot live in
//! memory. It is written machine-local and branch-independent — writing it
//! into the worktree would leave an untracked file behind the Stop gate,
//! which is exactly the failure #4669 records.
//!
//! The record is deliberately a single timestamp, not a list of what the
//! survey returned. The gate asks whether the PM looked, never what it found:
//! an Issue that belongs to no spec must still register (#4680 AC-4).

use std::{
    num::NonZeroU64,
    path::Path,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};

/// Issue #5349 AC-8: evidence for one PM Issue registration. The survey
/// timestamp remains the hook prerequisite; it cannot stand in for the
/// explicit owner/search/split decision carried by `issue.create`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueOwnerCheck {
    // The custom deserializer makes the key mandatory while allowing an
    // explicit null to record that no spec owns the area.
    #[serde(deserialize_with = "required_owner_spec")]
    pub owner_spec: Option<NonZeroU64>,
    #[serde(default)]
    pub search_queries: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

fn required_owner_spec<'de, D>(deserializer: D) -> Result<Option<NonZeroU64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<NonZeroU64>::deserialize(deserializer)
}

pub const OWNER_CHECK_RECOVERY: &str = r#"PM issue.create requires params.owner_check evidence. After issue.spec.list, search, read all addenda/corrections/comments, and check current develop/code/tests/merged PRs/closed Issues, update the existing owner's FR/AC or justify an independently deliverable child. No owner: {"owner_spec":null,"search_queries":["actual search query"]}. Child of an existing owner: {"owner_spec":3431,"reason":"why a separate child is necessary"}. Record the rationale and search queries in one to three lines."#;

impl IssueOwnerCheck {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .search_queries
            .iter()
            .any(|query| query.trim().is_empty())
        {
            return Err(format!(
                "owner_check.search_queries must contain nonempty strings. {OWNER_CHECK_RECOVERY}"
            ));
        }
        if self.owner_spec.is_none() && self.search_queries.is_empty() {
            return Err(format!("owner_check.search_queries is required when owner_spec is null. {OWNER_CHECK_RECOVERY}"));
        }
        if self.owner_spec.is_some()
            && self
                .reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
        {
            return Err(format!("owner_check.reason is required for a child of an existing owner. {OWNER_CHECK_RECOVERY}"));
        }
        Ok(())
    }
}

/// How long one survey covers. A PM cycle is five minutes and a registration
/// normally follows its survey within the same cycle, so an hour is generous
/// without letting yesterday's survey wave through today's Issue.
const SURVEY_FRESHNESS: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpecSurveyRecord {
    /// RFC 3339 instant of the most recent `issue.spec.list` this PM ran.
    surveyed_at: String,
}

/// Record that the PM has just surveyed the open `gwt-spec` Issues.
///
/// Failures are swallowed on purpose: a hook that cannot write its own
/// bookkeeping must not deny the operation it was observing.
pub fn record_survey(worktree_root: &Path) {
    let path = gwt_core::paths::gwt_pm_spec_survey_path(worktree_root);
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let record = SpecSurveyRecord {
        surveyed_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Ok(body) = serde_json::to_string(&record) {
        let _ = std::fs::write(&path, body);
    }
}

/// Whether a survey recorded within the freshness window (one hour) is on
/// file.
///
/// An unreadable or absent record answers `false`, which is the safe side for
/// a gate whose remedy is one read-only operation.
pub fn survey_is_fresh(worktree_root: &Path) -> bool {
    let path = gwt_core::paths::gwt_pm_spec_survey_path(worktree_root);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(record) = serde_json::from_str::<SpecSurveyRecord>(&body) else {
        return false;
    };
    let Ok(surveyed_at) = chrono::DateTime::parse_from_rfc3339(&record.surveyed_at) else {
        return false;
    };
    let surveyed_at = SystemTime::from(surveyed_at);
    SystemTime::now()
        .duration_since(surveyed_at)
        .is_ok_and(|elapsed| elapsed <= SURVEY_FRESHNESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;

    #[test]
    fn a_recorded_survey_reads_back_as_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("gwt-home"));
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        assert!(
            !survey_is_fresh(&worktree),
            "no record on file should not read as surveyed"
        );
        record_survey(&worktree);
        assert!(survey_is_fresh(&worktree));
    }

    #[test]
    fn a_survey_older_than_the_freshness_window_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("gwt-home"));
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let path = gwt_core::paths::gwt_pm_spec_survey_path(&worktree);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let stale =
            chrono::Utc::now() - chrono::Duration::seconds(SURVEY_FRESHNESS.as_secs() as i64 + 60);
        std::fs::write(
            &path,
            serde_json::to_string(&SpecSurveyRecord {
                surveyed_at: stale.to_rfc3339(),
            })
            .unwrap(),
        )
        .unwrap();

        assert!(!survey_is_fresh(&worktree));
    }

    #[test]
    fn an_unreadable_record_does_not_count_as_a_survey() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("gwt-home"));
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let path = gwt_core::paths::gwt_pm_spec_survey_path(&worktree);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();

        assert!(!survey_is_fresh(&worktree));
    }
}
