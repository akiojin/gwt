//! Differential sync of merged pull requests (SPEC #4093 FR-003, Issue #3821).
//!
//! The Issue Monitor needs to know which work branches merged. Until this
//! module it asked `gh pr list --state merged --limit 999` on every scan: one
//! GraphQL query whose point cost grows with the repository's merged-PR count,
//! and which by 2026-08-31 burned the whole hourly GraphQL budget in minutes.
//!
//! The replacement reads closed pull requests through REST (`core` budget, one
//! request per page regardless of size) ordered by `updated_at` descending and
//! stops at the `updated_at` watermark the previous sync recorded. GitHub bumps
//! `updated_at` on merge, so every PR merged since the last sync is on the
//! pages above the watermark. Merged rows accumulate in a machine-local store
//! shared by every process of the repository, and the caller always receives
//! the accumulated set — a settlement that failed its readback, or a row that
//! only later became a candidate, must still find its delivery on a later scan.
//!
//! Cost per sync is bounded by [`SYNC_MAX_PAGES_PER_RUN`] REST requests and
//! never spends GraphQL. Steady state is one request.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

use crate::pr_status::{
    run_cached_list_command_with_observed_at, run_gh_command, GhCliOutput, MergedPrDeliveries,
    MergedPrDelivery, MergedPrRow,
};

/// Rows per REST page (GitHub's maximum).
pub const SYNC_PAGE_SIZE: usize = 100;

/// Upper bound on REST requests one sync may spend. A cold store therefore
/// covers the `SYNC_MAX_PAGES_PER_RUN * SYNC_PAGE_SIZE` most recently updated
/// closed PRs; deeper history is not needed to reconcile current work.
pub const SYNC_MAX_PAGES_PER_RUN: usize = 10;

/// Newest merged PRs the store keeps, so its file stays bounded on a
/// repository that merges for years.
pub const STORE_MAX_RECORDS: usize = 2_000;

const STORE_SCHEMA_VERSION: u32 = 1;
const STORE_FILE: &str = "project-state/merged-pr-sync.json";

/// One merged pull request as the store remembers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedPrRecord {
    pub number: u64,
    pub head_ref: String,
    pub base_ref: Option<String>,
    pub merge_sha: Option<String>,
    pub merged_at: Option<String>,
    pub updated_at: String,
}

/// A coherent, bounded first REST page. Accumulated delivery rows cannot
/// prove an interval complete; only this frozen source observation can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedPrObservation {
    pub observed_at: DateTime<Utc>,
    pub newest_updated_at: Option<String>,
    pub oldest_updated_at: Option<String>,
    pub exhausted: bool,
    pub merged: Vec<MergedPrRecord>,
}

impl MergedPrObservation {
    /// Count PR numbers for a caller-supplied current interval `[start, end)`.
    /// The caller owns local-day/timezone conversion and the freshness limit.
    /// `None` means the frozen page cannot prove the requested count.
    pub fn landed_count(
        &self,
        base_ref: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        now: DateTime<Utc>,
        max_age: chrono::Duration,
    ) -> Option<usize> {
        if base_ref.trim().is_empty()
            || start > self.observed_at
            || self.observed_at > now
            || now >= end
            || max_age <= chrono::Duration::zero()
            || now.signed_duration_since(self.observed_at) >= max_age
            || self.merged.len() > SYNC_PAGE_SIZE
            || (self.exhausted && self.merged.len() == SYNC_PAGE_SIZE)
        {
            return None;
        }
        let (oldest, newest) = match (
            self.oldest_updated_at.as_deref(),
            self.newest_updated_at.as_deref(),
        ) {
            (None, None) => return (self.exhausted && self.merged.is_empty()).then_some(0),
            (Some(oldest), Some(newest)) => (oldest, newest),
            _ => return None,
        };
        let oldest = DateTime::parse_from_rfc3339(oldest).ok()?;
        let newest = DateTime::parse_from_rfc3339(newest).ok()?;
        // Equality cannot prove completeness: a full page can hide another
        // PR with the same second-level updated_at as the inclusive start.
        if oldest > newest || newest > now || (!self.exhausted && oldest >= start) {
            return None;
        }
        let mut numbers = HashSet::new();
        let mut previous = None;
        let mut count = 0;
        for record in &self.merged {
            let base = record
                .base_ref
                .as_deref()
                .filter(|s| !s.trim().is_empty())?;
            let updated_at = DateTime::parse_from_rfc3339(&record.updated_at).ok()?;
            let merged_at = DateTime::parse_from_rfc3339(record.merged_at.as_deref()?).ok()?;
            if record.number == 0
                || !numbers.insert(record.number)
                || record.head_ref.trim().is_empty()
                || updated_at < oldest
                || updated_at > newest
                || merged_at > updated_at
                || previous.is_some_and(|previous| previous < updated_at)
            {
                return None;
            }
            previous = Some(updated_at);
            if base == base_ref && merged_at >= start && merged_at < end {
                count += 1;
            }
        }
        Some(count)
    }
}

/// The persisted sync state: the watermark plus every merged PR seen so far,
/// keyed by PR number.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedPrStore {
    #[serde(default)]
    pub schema_version: u32,
    /// `updated_at` of the newest closed PR a sync has seen. Rows at or below
    /// it were already read. GitHub emits UTC `YYYY-MM-DDTHH:MM:SSZ`, so the
    /// timestamps compare as strings.
    #[serde(default)]
    pub watermark: Option<String>,
    #[serde(default)]
    pub merged: BTreeMap<u64, MergedPrRecord>,
    /// Old stores have no completeness or freshness witness.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_observation"
    )]
    pub observation: Option<MergedPrObservation>,
}

fn deserialize_observation<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<MergedPrObservation>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(raw).ok())
}

/// What one sync did, for callers that report cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPrSyncOutcome {
    pub deliveries: MergedPrDeliveries,
    /// REST requests spent by this sync.
    pub requests: usize,
    /// `false` when the page budget ran out before the watermark was reached,
    /// so older updates were skipped this time.
    pub complete: bool,
}

/// The store file for `repo_path`'s project, next to the Issue Monitor state.
pub fn store_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join(STORE_FILE)
}

/// Sync merged PRs for `repo_path` through the budget-gated `gh` spawn and
/// return the accumulated deliveries. A failed request returns `Err` without
/// touching the store, so a transient failure never looks like "nothing
/// merged".
pub fn sync_merged_pr_deliveries(repo_path: &Path) -> Result<MergedPrDeliveries> {
    let store_path = store_path_for_repo_path(repo_path);
    sync_with(&store_path, repo_path, run_gh_command).map(|outcome| outcome.deliveries)
}

/// Short-lived inventory cache for non-destructive Monitor reconciliation.
pub fn sync_merged_pr_deliveries_for_monitor(repo_path: &Path) -> Result<MergedPrDeliveries> {
    let store_path = store_path_for_repo_path(repo_path);
    sync_with_observed_at(
        &store_path,
        repo_path,
        run_cached_list_command_with_observed_at,
    )
    .map(|outcome| outcome.deliveries)
}

/// Injectable core of [`sync_merged_pr_deliveries`].
pub(crate) fn sync_with<F>(
    store_path: &Path,
    repo_path: &Path,
    mut run_gh: F,
) -> Result<MergedPrSyncOutcome>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    sync_with_observed_at(store_path, repo_path, |repo_path, args| {
        // Acquisition starts before the live request; response reuse must
        // instead supply the cache's original acquisition time.
        let observed_at = Utc::now();
        run_gh(repo_path, args).map(|output| (observed_at, output))
    })
}

pub(crate) fn sync_with_observed_at<F>(
    store_path: &Path,
    repo_path: &Path,
    mut run_gh: F,
) -> Result<MergedPrSyncOutcome>
where
    F: FnMut(&Path, &[&str]) -> Result<(DateTime<Utc>, GhCliOutput)>,
{
    let watermark = load_store(store_path).watermark;
    let mut fetched = Vec::new();
    let mut observation = None;
    let mut newest: Option<String> = None;
    let mut requests = 0;
    let mut complete = false;
    for page in 1..=SYNC_MAX_PAGES_PER_RUN {
        let endpoint = closed_pulls_endpoint(page);
        let (observed_at, output) = run_gh(repo_path, &["api", &endpoint])?;
        requests += 1;
        if !output.success {
            return Err(GwtError::Git(format!(
                "gh api pulls closed page {page}: {}",
                output.stderr.trim()
            )));
        }
        let raw: Vec<serde_json::Value> = serde_json::from_str(&output.stdout)
            .map_err(|e| GwtError::Other(format!("gh api pulls closed JSON: {e}")))?;
        let count = raw.len();
        let rows = parse_closed_pull_rows(&raw);
        if page == 1 {
            observation = observation_from_page(&raw, &rows, observed_at);
        }
        let mut oldest: Option<String> = None;
        for row in rows {
            if newest
                .as_deref()
                .is_none_or(|current| row.updated_at.as_str() > current)
            {
                newest = Some(row.updated_at.clone());
            }
            if oldest
                .as_deref()
                .is_none_or(|current| row.updated_at.as_str() < current)
            {
                oldest = Some(row.updated_at.clone());
            }
            if let Some(record) = row.into_merged_record() {
                fetched.push(record);
            }
        }
        if count < SYNC_PAGE_SIZE {
            complete = true;
            break;
        }
        if let (Some(oldest), Some(watermark)) = (&oldest, &watermark) {
            if oldest <= watermark {
                complete = true;
                break;
            }
        }
    }
    // Re-read before writing: another process (GUI and daemon share the
    // store) may have synced meanwhile, and its rows must not be lost.
    let mut store = load_store(store_path);
    store.schema_version = STORE_SCHEMA_VERSION;
    store.observation = observation;
    for record in fetched {
        store.merged.insert(record.number, record);
    }
    if let Some(newest) = newest {
        if store
            .watermark
            .as_deref()
            .is_none_or(|current| newest.as_str() > current)
        {
            store.watermark = Some(newest);
        }
    }
    while store.merged.len() > STORE_MAX_RECORDS {
        store.merged.pop_first();
    }
    // Bookkeeping must never fail the read it serves: an unwritable store only
    // means the next sync re-reads the same pages.
    let _ = save_store(store_path, &store);
    Ok(MergedPrSyncOutcome {
        deliveries: deliveries_from_store(&store),
        requests,
        complete,
    })
}

fn closed_pulls_endpoint(page: usize) -> String {
    format!(
        "repos/{{owner}}/{{repo}}/pulls?state=closed&sort=updated&direction=desc&per_page={SYNC_PAGE_SIZE}&page={page}"
    )
}

fn deliveries_from_store(store: &MergedPrStore) -> MergedPrDeliveries {
    MergedPrDeliveries::from_rows(store.merged.values().map(|record| MergedPrRow {
        head_ref: record.head_ref.clone(),
        delivery: Some(MergedPrDelivery {
            number: record.number,
            merge_sha: record.merge_sha.clone(),
            base_ref: record.base_ref.clone(),
            merged_at: record.merged_at.clone(),
        }),
    }))
}

fn load_store(path: &Path) -> MergedPrStore {
    std::fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

fn save_store(path: &Path, store: &MergedPrStore) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(store).map_err(std::io::Error::other)?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// One row of `GET /repos/{owner}/{repo}/pulls?state=closed`.
struct ClosedPullRow {
    number: u64,
    head_ref: String,
    base_ref: Option<String>,
    merge_sha: Option<String>,
    merged_at: Option<String>,
    updated_at: String,
}

impl ClosedPullRow {
    /// Closed-without-merge rows carry no `merged_at` and are dropped.
    fn into_merged_record(self) -> Option<MergedPrRecord> {
        let merged_at = self.merged_at?;
        Some(MergedPrRecord {
            number: self.number,
            head_ref: self.head_ref,
            base_ref: self.base_ref,
            merge_sha: self.merge_sha,
            merged_at: Some(merged_at),
            updated_at: self.updated_at,
        })
    }
}

/// Rows without a number, head ref, or `updated_at` cannot be reconciled and
/// are skipped.
fn parse_closed_pull_rows(arr: &[serde_json::Value]) -> Vec<ClosedPullRow> {
    let text = |value: &serde_json::Value, path: &[&str]| {
        let mut cursor = value;
        for key in path {
            cursor = cursor.get(key)?;
        }
        cursor
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(String::from)
    };
    arr.iter()
        .filter_map(|value| {
            Some(ClosedPullRow {
                number: value.get("number").and_then(serde_json::Value::as_u64)?,
                head_ref: text(value, &["head", "ref"])?,
                base_ref: text(value, &["base", "ref"]),
                merge_sha: text(value, &["merge_commit_sha"]),
                merged_at: text(value, &["merged_at"]),
                updated_at: text(value, &["updated_at"])?,
            })
        })
        .collect()
}

fn observation_from_page(
    raw: &[serde_json::Value],
    rows: &[ClosedPullRow],
    observed_at: DateTime<Utc>,
) -> Option<MergedPrObservation> {
    if raw.len() > SYNC_PAGE_SIZE || raw.len() != rows.len() {
        return None;
    }
    let mut numbers = HashSet::new();
    let mut previous = None;
    let mut merged = Vec::new();
    for (value, row) in raw.iter().zip(rows) {
        let updated_at = DateTime::parse_from_rfc3339(&row.updated_at).ok()?;
        if row.number == 0
            || !numbers.insert(row.number)
            || row.base_ref.is_none()
            || previous.is_some_and(|previous| previous < updated_at)
        {
            return None;
        }
        previous = Some(updated_at);
        match value.get("merged_at")? {
            serde_json::Value::Null => {}
            serde_json::Value::String(_) => {
                let merged_at = row.merged_at.as_deref()?;
                if DateTime::parse_from_rfc3339(merged_at).ok()? > updated_at {
                    return None;
                }
                merged.push(MergedPrRecord {
                    number: row.number,
                    head_ref: row.head_ref.clone(),
                    base_ref: row.base_ref.clone(),
                    merge_sha: row.merge_sha.clone(),
                    merged_at: row.merged_at.clone(),
                    updated_at: row.updated_at.clone(),
                });
            }
            _ => return None,
        }
    }
    Some(MergedPrObservation {
        observed_at,
        newest_updated_at: rows.first().map(|row| row.updated_at.clone()),
        oldest_updated_at: rows.last().map(|row| row.updated_at.clone()),
        exhausted: raw.len() < SYNC_PAGE_SIZE,
        merged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn stamp(minutes: i64) -> String {
        chrono::DateTime::from_timestamp(1_760_000_000 + minutes * 60, 0)
            .expect("timestamp")
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    /// A PR merged into `base`; higher numbers are newer.
    fn merged_row(number: u64, base: &str) -> serde_json::Value {
        serde_json::json!({
            "number": number,
            "state": "closed",
            "head": {"ref": format!("work/issue-{number}")},
            "base": {"ref": base},
            "merged_at": stamp(number as i64),
            "merge_commit_sha": format!("sha{number}"),
            "updated_at": stamp(number as i64),
        })
    }

    /// Fake REST endpoint over `rows` (newest first) that records every call.
    struct FakeGitHub {
        rows: Vec<serde_json::Value>,
        calls: Vec<String>,
        fail_page: Option<usize>,
    }

    impl FakeGitHub {
        fn merged(numbers: impl DoubleEndedIterator<Item = u64>) -> Self {
            Self::with_rows(numbers.rev().map(|n| merged_row(n, "develop")).collect())
        }

        fn with_rows(rows: Vec<serde_json::Value>) -> Self {
            Self {
                rows,
                calls: Vec::new(),
                fail_page: None,
            }
        }

        fn serve(&mut self, args: &[&str]) -> Result<GhCliOutput> {
            self.calls.push(args.join(" "));
            assert_eq!(
                args[0], "api",
                "GraphQL `gh pr list` must never be spawned: {args:?}"
            );
            let page: usize = args[1].rsplit("&page=").next().unwrap().parse().unwrap();
            if self.fail_page == Some(page) {
                return Ok(GhCliOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "HTTP 502".to_string(),
                });
            }
            let start = (page - 1) * SYNC_PAGE_SIZE;
            let slice = self.rows.get(start..).unwrap_or(&[]);
            let slice = &slice[..slice.len().min(SYNC_PAGE_SIZE)];
            Ok(GhCliOutput {
                success: true,
                stdout: serde_json::to_string(slice).unwrap(),
                stderr: String::new(),
            })
        }

        fn sync(&mut self, store: &Path) -> Result<MergedPrSyncOutcome> {
            let calls_before = self.calls.len();
            let outcome = sync_with(store, Path::new("/repo"), |_, args| self.serve(args));
            let spent = self.calls.len() - calls_before;
            assert!(
                self.calls[calls_before..]
                    .iter()
                    .all(|call| call.starts_with("api repos/{owner}/{repo}/pulls?state=closed")),
                "only the REST closed-pulls endpoint may be spent: {:?}",
                &self.calls[calls_before..]
            );
            outcome.inspect(|outcome| assert_eq!(outcome.requests, spent))
        }
    }

    fn store_in(temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join("project-state/merged-pr-sync.json")
    }

    #[test]
    fn cold_sync_over_1500_merged_prs_spends_at_most_the_page_budget_and_no_graphql() {
        // SPEC #4093 AC-2: the scan cost must not scale with the merged-PR
        // count. 1,500 merged PRs cost at most SYNC_MAX_PAGES_PER_RUN REST
        // requests, and `gh pr list` (GraphQL) is never spawned.
        let temp = tempfile::tempdir().unwrap();
        let mut github = FakeGitHub::merged(1..=1500);
        let outcome = github.sync(&store_in(&temp)).unwrap();
        assert!(
            outcome.requests <= SYNC_MAX_PAGES_PER_RUN,
            "{}",
            outcome.requests
        );
        assert!(!outcome.complete, "1,500 rows exceed one run's page budget");
        let covered = SYNC_MAX_PAGES_PER_RUN * SYNC_PAGE_SIZE;
        assert_eq!(outcome.deliveries.deliveries.len(), covered);
        assert!(outcome.deliveries.branches.contains("work/issue-1500"));
        assert!(outcome
            .deliveries
            .branches
            .contains(&format!("work/issue-{}", 1500 - covered + 1)));
        assert_eq!(
            outcome.deliveries.deliveries["work/issue-1500"].number,
            1500
        );
        assert_eq!(
            outcome.deliveries.deliveries["work/issue-1500"]
                .merge_sha
                .as_deref(),
            Some("sha1500")
        );
    }

    #[test]
    fn steady_state_sync_spends_one_request_and_keeps_accumulated_deliveries() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(&temp);
        let mut github = FakeGitHub::merged(1..=250);
        assert_eq!(github.sync(&store).unwrap().requests, 3);

        let unchanged = github.sync(&store).unwrap();
        assert_eq!(unchanged.requests, 1, "nothing new stops at the first page");
        assert!(unchanged.complete);
        assert_eq!(unchanged.deliveries.deliveries.len(), 250);

        github.rows.insert(0, merged_row(251, "develop"));
        let grown = github.sync(&store).unwrap();
        assert_eq!(grown.requests, 1);
        assert_eq!(grown.deliveries.deliveries.len(), 251);
        assert_eq!(grown.deliveries.deliveries["work/issue-251"].number, 251);
        assert!(
            grown.deliveries.deliveries.contains_key("work/issue-1"),
            "earlier deliveries stay visible so a deferred settlement can still find them"
        );
    }

    #[test]
    fn sync_pages_until_the_watermark_is_reached() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(&temp);
        let mut github = FakeGitHub::merged(1..=250);
        github.sync(&store).unwrap();

        for number in 251..=400 {
            github.rows.insert(0, merged_row(number, "develop"));
        }
        let outcome = github.sync(&store).unwrap();
        assert_eq!(
            outcome.requests, 2,
            "150 new rows span two pages; the second page reaches the watermark"
        );
        assert!(outcome.complete);
        assert_eq!(outcome.deliveries.deliveries.len(), 400);
    }

    #[test]
    fn failed_page_returns_error_and_leaves_the_store_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(&temp);
        let mut github = FakeGitHub::merged(1..=50);
        github.sync(&store).unwrap();
        let persisted = std::fs::read(&store).unwrap();

        github.rows.insert(0, merged_row(51, "develop"));
        github.fail_page = Some(1);
        let error = github.sync(&store).unwrap_err().to_string();
        assert!(error.contains("HTTP 502"), "{error}");
        assert_eq!(std::fs::read(&store).unwrap(), persisted);

        github.fail_page = None;
        let recovered = github.sync(&store).unwrap();
        assert_eq!(recovered.requests, 1);
        assert_eq!(recovered.deliveries.deliveries.len(), 51);
    }

    #[test]
    fn rest_rows_follow_the_delivery_policy() {
        // Closed-without-merge rows are not merged branches; a merge into a
        // base other than develop is a merged branch but not a delivery
        // (Issue #3917).
        let temp = tempfile::tempdir().unwrap();
        let mut unmerged = merged_row(3, "develop");
        unmerged["merged_at"] = serde_json::Value::Null;
        unmerged["merge_commit_sha"] = serde_json::Value::Null;
        let mut github = FakeGitHub::with_rows(vec![
            unmerged,
            merged_row(2, "main"),
            merged_row(1, "develop"),
        ]);
        let outcome = github.sync(&store_in(&temp)).unwrap();
        assert_eq!(
            outcome.deliveries.branches,
            ["work/issue-1", "work/issue-2"]
                .into_iter()
                .map(String::from)
                .collect()
        );
        assert_eq!(
            outcome
                .deliveries
                .deliveries
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["work/issue-1"]
        );
        let delivery = &outcome.deliveries.deliveries["work/issue-1"];
        assert_eq!(delivery.base_ref.as_deref(), Some("develop"));
        assert_eq!(delivery.merged_at.as_deref(), Some(stamp(1).as_str()));
    }

    #[test]
    fn successful_sync_freezes_only_the_first_raw_page() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(&temp);
        let mut github = FakeGitHub::merged(1..=150);
        assert_eq!(github.sync(&store).unwrap().requests, 2);

        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store).unwrap()).unwrap();
        let observation = &persisted["observation"];
        assert!(observation["observed_at"].is_string());
        assert_eq!(observation["newest_updated_at"], stamp(150));
        assert_eq!(observation["oldest_updated_at"], stamp(51));
        assert_eq!(observation["exhausted"], false);
        let merged = observation["merged"].as_array().unwrap();
        assert_eq!(merged.len(), SYNC_PAGE_SIZE);
        assert_eq!(merged[0]["number"], 150);
        assert_eq!(merged[SYNC_PAGE_SIZE - 1]["number"], 51);
    }

    #[test]
    fn malformed_row_does_not_make_a_full_raw_page_look_exhausted() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(&temp);
        let mut github = FakeGitHub::merged(1..=101);
        github.rows[0].as_object_mut().unwrap().remove("updated_at");

        let outcome = github.sync(&store).unwrap();
        assert_eq!(outcome.requests, 2);
        assert_eq!(outcome.deliveries.deliveries.len(), 100);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store).unwrap()).unwrap();
        assert!(persisted["observation"].is_null());
    }

    #[test]
    fn landed_count_uses_caller_local_day_base_and_distinct_pr_numbers() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(&temp);
        // Midnight in UTC+09 is 15:00Z on the previous UTC date.
        let start = instant("2026-10-27T15:00:00Z");
        let end = instant("2026-10-28T15:00:00Z");
        let now = instant("2026-10-28T03:00:00Z");
        let mut first = merged_row(4, "develop");
        first["head"]["ref"] = "work/shared".into();
        first["merged_at"] = "2026-10-28T02:59:59Z".into();
        first["updated_at"] = "2026-10-28T02:59:59Z".into();
        let mut boundary = merged_row(3, "develop");
        boundary["head"]["ref"] = "work/shared".into();
        boundary["merged_at"] = start.to_rfc3339().into();
        boundary["updated_at"] = start.to_rfc3339().into();
        let mut main = merged_row(2, "main");
        main["merged_at"] = start.to_rfc3339().into();
        main["updated_at"] = start.to_rfc3339().into();
        let mut yesterday = merged_row(1, "develop");
        yesterday["merged_at"] = "2026-10-27T14:59:59Z".into();
        yesterday["updated_at"] = "2026-10-27T14:59:59Z".into();
        let mut github = FakeGitHub::with_rows(vec![first, boundary, main, yesterday]);
        sync_with_observed_at(&store, Path::new("/repo"), |_, args| {
            // The first merge occurred during the live request, after acquisition.
            github
                .serve(args)
                .map(|output| (now - chrono::Duration::seconds(10), output))
        })
        .unwrap();
        let observation = load_store(&store).observation.unwrap();
        let freshness = chrono::Duration::seconds(120);
        assert_eq!(
            observation.landed_count("develop", start, end, now, freshness),
            Some(2)
        );
        assert_eq!(
            observation.landed_count("main", start, end, now, freshness),
            Some(1)
        );
        assert_eq!(
            observation.landed_count("release", start, end, now, freshness),
            Some(0)
        );
        let old: MergedPrStore = serde_json::from_str(r#"{"merged":{}}"#).unwrap();
        assert!(old.observation.is_none());

        let mut future_closed = merged_row(5, "develop");
        future_closed["merged_at"] = serde_json::Value::Null;
        future_closed["updated_at"] = (now + chrono::Duration::seconds(1)).to_rfc3339().into();
        github.rows.insert(0, future_closed);
        sync_with_observed_at(&store, Path::new("/repo"), |_, args| {
            github.serve(args).map(|output| (now, output))
        })
        .unwrap();
        assert_eq!(
            load_store(&store)
                .observation
                .unwrap()
                .landed_count("develop", start, end, now, freshness),
            None,
            "a future unmerged raw row invalidates the source clock too"
        );
    }

    #[test]
    fn landed_count_requires_strict_coverage_and_valid_frozen_rows() {
        let start = instant("2026-10-27T15:00:00Z");
        let end = start + chrono::Duration::days(1);
        let now = start + chrono::Duration::hours(1);
        let freshness = chrono::Duration::seconds(120);
        let mut observation = MergedPrObservation {
            observed_at: now,
            newest_updated_at: Some(start.to_rfc3339()),
            oldest_updated_at: Some(start.to_rfc3339()),
            exhausted: false,
            merged: vec![],
        };
        // A full page ending exactly at midnight can hide another PR tied
        // at that second. A short page has exhausted the raw history.
        assert_eq!(
            observation.landed_count("develop", start, end, now, freshness),
            None
        );
        observation.exhausted = true;
        assert_eq!(
            observation.landed_count("develop", start, end, now, freshness),
            Some(0)
        );
        observation.exhausted = false;
        observation.oldest_updated_at = Some((start - chrono::Duration::seconds(1)).to_rfc3339());
        assert_eq!(
            observation.landed_count("develop", start, end, now, freshness),
            Some(0)
        );

        let record = MergedPrRecord {
            number: 1,
            head_ref: "work/shared".into(),
            base_ref: Some("develop".into()),
            merge_sha: None,
            merged_at: Some(start.to_rfc3339()),
            updated_at: start.to_rfc3339(),
        };
        observation.merged = vec![record.clone(), record.clone()];
        assert_eq!(
            observation.landed_count("develop", start, end, now, freshness),
            None,
            "duplicate PR numbers cannot prove a count"
        );
        observation.merged.pop();
        observation.merged[0].base_ref = None;
        assert_eq!(
            observation.landed_count("develop", start, end, now, freshness),
            None,
            "a missing base cannot be mistaken for a different base"
        );
        observation.exhausted = true;
        observation.merged = (1..=SYNC_PAGE_SIZE as u64)
            .map(|number| MergedPrRecord {
                number,
                ..record.clone()
            })
            .collect();
        assert_eq!(
            observation.landed_count("develop", start, end, now, freshness),
            None,
            "a full merged page cannot claim raw history exhaustion"
        );

        let corrupt_receipt = serde_json::json!({
            "merged": {"1": record},
            "observation": {"observed_at": "invalid"},
        });
        let recovered: MergedPrStore = serde_json::from_value(corrupt_receipt)
            .expect("a corrupt optional observation must preserve accumulated deliveries");
        assert!(recovered.observation.is_none());
        assert_eq!(recovered.merged.len(), 1);
        assert!(deliveries_from_store(&recovered)
            .deliveries
            .contains_key("work/shared"));
    }

    #[test]
    fn cached_observation_keeps_acquisition_age_and_failed_refresh_preserves_store() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(&temp);
        let cache = temp.path().join("cache");
        let start = instant("2026-10-27T15:00:00Z");
        let acquired = start + chrono::Duration::hours(1);
        let ttl = crate::gh_rest::LIST_CACHE_TTL_SECS;
        let mut fetches = 0;
        for elapsed in [0, ttl / 2, ttl] {
            let at = acquired + chrono::Duration::seconds(elapsed);
            let before = std::fs::read(&store).ok();
            let result = sync_with_observed_at(&store, Path::new("/repo"), |_, args| {
                let (observed_at, stdout) = crate::gh_rest::cached_list_response_at_with(
                    &cache,
                    &args.join(" "),
                    at,
                    ttl,
                    || {
                        fetches += 1;
                        if elapsed == 0 {
                            Ok("[]".into())
                        } else {
                            Err("HTTP 502".into())
                        }
                    },
                )
                .map_err(GwtError::Git)?;
                Ok((
                    observed_at,
                    GhCliOutput {
                        success: true,
                        stdout,
                        stderr: String::new(),
                    },
                ))
            });
            if elapsed < ttl {
                result.unwrap();
                assert_eq!(
                    load_store(&store).observation.unwrap().observed_at,
                    acquired
                );
            } else {
                assert!(result.unwrap_err().to_string().contains("HTTP 502"));
                assert_eq!(std::fs::read(&store).unwrap(), before.unwrap());
            }
        }
        assert_eq!(
            fetches, 2,
            "cache reuse must not issue another live request"
        );
        let observation = load_store(&store).observation.unwrap();
        let end = start + chrono::Duration::days(1);
        let freshness = chrono::Duration::seconds(ttl);
        assert_eq!(
            observation.landed_count("develop", start, end, acquired, freshness),
            Some(0)
        );
        assert_eq!(
            observation.landed_count("develop", start, end, acquired + freshness, freshness),
            None
        );
    }
}
