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
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

use crate::pr_status::{
    run_gh_command, GhCliOutput, MergedPrDeliveries, MergedPrDelivery, MergedPrRow,
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

/// Injectable core of [`sync_merged_pr_deliveries`].
pub(crate) fn sync_with<F>(
    store_path: &Path,
    repo_path: &Path,
    mut run_gh: F,
) -> Result<MergedPrSyncOutcome>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let watermark = load_store(store_path).watermark;
    let mut fetched = Vec::new();
    let mut newest: Option<String> = None;
    let mut requests = 0;
    let mut complete = false;
    for page in 1..=SYNC_MAX_PAGES_PER_RUN {
        let endpoint = closed_pulls_endpoint(page);
        let output = run_gh(repo_path, &["api", &endpoint])?;
        requests += 1;
        if !output.success {
            return Err(GwtError::Git(format!(
                "gh api pulls closed page {page}: {}",
                output.stderr.trim()
            )));
        }
        let rows = parse_closed_pull_rows(&output.stdout)?;
        let count = rows.len();
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
fn parse_closed_pull_rows(json: &str) -> Result<Vec<ClosedPullRow>> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| GwtError::Other(format!("gh api pulls closed JSON: {e}")))?;
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
    Ok(arr
        .iter()
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
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
