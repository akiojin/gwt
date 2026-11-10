//! Codex model catalog (Issue #6933).
//!
//! Codex writes the model list its own picker shows to
//! `$CODEX_HOME/models_cache.json`. The rows whose `visibility` is `list`,
//! ordered by `priority`, are exactly the "Select Model and Effort" rows. gwt
//! reads that file so the Launch Wizard and Agents settings follow Codex
//! without a gwt release, and falls back to the bundled snapshot below when
//! the cache is missing, unreadable, or not in the expected shape.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};

use chrono::{DateTime, Duration, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

/// One Codex picker row with the reasoning stops the model accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexModelRow {
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub default_effort: String,
    /// Supported reasoning efforts in Codex's own order.
    pub efforts: Vec<String>,
}

const FULL_LADDER: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];

// Bundled fallback: the `visibility: "list"` rows of the 2026-11-09
// codex-cli 0.167.0 models cache, in priority order.
// (slug, display_name, description, default effort, highest effort)
const SNAPSHOT: [(&str, &str, &str, &str, &str); 8] = [
    (
        "gpt-6.1-sol",
        "GPT-6.1-Sol",
        "Latest workhorse model for coding and everyday work.",
        "low",
        "ultra",
    ),
    (
        "gpt-6-astra",
        "GPT-6-Astra",
        "Frontier intelligence for the most demanding work.",
        "medium",
        "ultra",
    ),
    (
        "gpt-6.1-terra",
        "GPT-6.1 Terra",
        "GPT-6.1 Terra Codex model.",
        "low",
        "ultra",
    ),
    (
        "gpt-6-sol",
        "GPT-6-Sol",
        "Previous generation workhorse model.",
        "medium",
        "ultra",
    ),
    (
        "gpt-6-luna",
        "GPT-6-Luna",
        "Fast and affordable model for easier tasks.",
        "medium",
        "max",
    ),
    (
        "gpt-5.6-sol",
        "GPT-5.6-Sol",
        "Older generation workhorse model.",
        "low",
        "ultra",
    ),
    (
        "gpt-5.6-terra",
        "GPT-5.6-Terra",
        "Older balanced model for straightforward work.",
        "medium",
        "ultra",
    ),
    (
        "gpt-5.6-luna",
        "GPT-5.6-Luna",
        "Older fast and efficient model.",
        "medium",
        "max",
    ),
];

/// The bundled snapshot rows.
pub fn snapshot_rows() -> Vec<CodexModelRow> {
    SNAPSHOT
        .iter()
        .map(|(slug, name, description, default, max)| {
            let end = FULL_LADDER
                .iter()
                .position(|effort| effort == max)
                .map_or(FULL_LADDER.len(), |index| index + 1);
            CodexModelRow {
                slug: (*slug).to_string(),
                display_name: (*name).to_string(),
                description: (*description).to_string(),
                default_effort: (*default).to_string(),
                efforts: FULL_LADDER[..end].iter().map(|e| e.to_string()).collect(),
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct CacheFile {
    models: Vec<CacheModel>,
}

#[derive(Deserialize)]
struct CacheModel {
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supported_reasoning_levels: Vec<CacheEffort>,
    visibility: String,
    #[serde(default)]
    priority: Option<i64>,
}

#[derive(Deserialize)]
struct CacheEffort {
    effort: String,
}

/// Parse a Codex `models_cache.json` body into the visible picker rows.
///
/// Returns `None` when the body is not the expected shape. A valid empty
/// list stays empty: fallback must not resurrect explicitly hidden models.
pub fn parse_models_cache(body: &str) -> Option<Vec<CodexModelRow>> {
    let cache: CacheFile = serde_json::from_str(body).ok()?;
    let mut listed: Vec<CacheModel> = cache
        .models
        .into_iter()
        .filter(|model| model.visibility == "list")
        .filter(|model| !model.slug.trim().is_empty())
        .collect();
    // Stable sort keeps file order for equal or missing priorities.
    listed.sort_by_key(|model| model.priority.unwrap_or(i64::MAX));
    let rows: Vec<CodexModelRow> = listed
        .into_iter()
        .map(|model| {
            let efforts: Vec<String> = model
                .supported_reasoning_levels
                .into_iter()
                .map(|level| level.effort)
                .filter(|effort| !effort.trim().is_empty())
                .collect();
            let default_effort = model
                .default_reasoning_level
                .filter(|level| efforts.contains(level))
                .or_else(|| efforts.first().cloned())
                .unwrap_or_default();
            CodexModelRow {
                display_name: model
                    .display_name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| model.slug.clone()),
                description: model.description.unwrap_or_default(),
                slug: model.slug,
                default_effort,
                efforts,
            }
        })
        .collect();
    Some(rows)
}

/// Where Codex keeps its models cache.
pub fn models_cache_path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        test_override::path()
    }
    #[cfg(not(test))]
    gwt_core::usage::codex::codex_home().map(|home| home.join("models_cache.json"))
}

/// Visible rows read from `path`, or `None` when the cache cannot be used.
pub fn rows_from_cache_file(path: &Path) -> Option<Vec<CodexModelRow>> {
    parse_models_cache(&std::fs::read_to_string(path).ok()?)
}

type CacheKey = (PathBuf, SystemTime, u64);
static MEMO: Mutex<Option<(CacheKey, Arc<Vec<CodexModelRow>>)>> = Mutex::new(None);

/// Rows from Codex's cache when usable (`Some`), otherwise `None`.
///
/// Re-reads the file only when its path, mtime, or size changed, because the
/// Wizard asks for the catalog many times per render.
pub fn cached_rows() -> Option<Arc<Vec<CodexModelRow>>> {
    let path = models_cache_path()?;
    let metadata = std::fs::metadata(&path).ok()?;
    let key = (path.clone(), metadata.modified().ok()?, metadata.len());
    let mut memo = MEMO.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some((memo_key, rows)) = memo.as_ref() {
        if *memo_key == key {
            return Some(rows.clone());
        }
    }
    let rows = Arc::new(rows_from_cache_file(&path)?);
    *memo = Some((key, rows.clone()));
    Some(rows)
}

/// The Codex model rows gwt offers: the cache when usable, else the snapshot.
pub fn current_rows() -> Arc<Vec<CodexModelRow>> {
    cached_rows().unwrap_or_else(|| Arc::new(snapshot_rows()))
}

/// Differences between the effective list, the bundled snapshot, and saved
/// profile models.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogDiff {
    /// Listed by Codex but missing from the bundled snapshot.
    pub added: Vec<String>,
    /// In the bundled snapshot but no longer listed by Codex.
    pub removed: Vec<String>,
    /// Both lists hold the same shared models in a different order.
    pub order_changed: bool,
    /// Saved Codex profile models that the effective list no longer offers.
    pub missing_saved_models: Vec<String>,
}

impl CatalogDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && !self.order_changed
            && self.missing_saved_models.is_empty()
    }

    fn report_body(&self, effective: &[CodexModelRow]) -> String {
        let mut body = String::from("# Codex model list changed\n\n");
        if !self.added.is_empty() {
            body.push_str(&format!(
                "- Added by Codex (not in gwt's bundled snapshot): {}\n",
                self.added.join(", ")
            ));
        }
        if !self.removed.is_empty() {
            body.push_str(&format!(
                "- No longer offered by Codex (still in gwt's bundled snapshot): {}\n",
                self.removed.join(", ")
            ));
        }
        // Include every row's position, including newly added models moving
        // relative to the snapshot's rows.
        body.push_str(&format!(
            "- Codex model order: {}\n",
            effective
                .iter()
                .map(|row| row.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        if !self.missing_saved_models.is_empty() {
            body.push_str(&format!(
                "- Saved launch profiles use models Codex no longer offers: {}. \
                 They are not replaced automatically; update them in Agents settings.\n",
                self.missing_saved_models.join(", ")
            ));
        }
        body
    }
}

/// Compare the effective list with the snapshot and the saved models.
pub fn diff_catalog(
    effective: &[CodexModelRow],
    snapshot: &[CodexModelRow],
    saved_models: &[String],
) -> CatalogDiff {
    let effective_slugs: Vec<&str> = effective.iter().map(|row| row.slug.as_str()).collect();
    let snapshot_slugs: Vec<&str> = snapshot.iter().map(|row| row.slug.as_str()).collect();
    let added = effective_slugs
        .iter()
        .filter(|slug| !snapshot_slugs.contains(slug))
        .map(|slug| slug.to_string())
        .collect();
    let removed = snapshot_slugs
        .iter()
        .filter(|slug| !effective_slugs.contains(slug))
        .map(|slug| slug.to_string())
        .collect();
    let shared_effective: Vec<&str> = effective_slugs
        .iter()
        .copied()
        .filter(|slug| snapshot_slugs.contains(slug))
        .collect();
    let shared_snapshot: Vec<&str> = snapshot_slugs
        .iter()
        .copied()
        .filter(|slug| effective_slugs.contains(slug))
        .collect();
    let mut missing_saved_models: Vec<String> = saved_models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty() && !effective_slugs.contains(model))
        .map(str::to_string)
        .collect();
    missing_saved_models.sort();
    missing_saved_models.dedup();
    CatalogDiff {
        added,
        removed,
        order_changed: shared_effective != shared_snapshot,
        missing_saved_models,
    }
}

const CHECK_INTERVAL_DAYS: i64 = 7;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CheckState {
    #[serde(default)]
    last_checked_at: Option<DateTime<Utc>>,
}

fn check_state_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("project-state/codex-model-check.json")
}

/// Outcome of one weekly check call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeeklyCheckOutcome {
    /// Fewer than seven days since the previous check.
    NotDue,
    /// Checked; nothing differs.
    NoDiff,
    /// Checked; the same difference was already reported.
    AlreadyReported,
    /// Checked; one PM report was posted.
    Reported,
}

/// Run the weekly Codex model check for one project.
///
/// `saved_models` are the models of the project's saved Codex launch
/// profiles. At most one PM report is posted per distinct difference.
pub fn run_weekly_check(
    repo_path: &Path,
    saved_models: &[String],
    now: DateTime<Utc>,
) -> gwt_core::Result<WeeklyCheckOutcome> {
    let state_path = check_state_path(repo_path);
    std::fs::create_dir_all(state_path.parent().expect("project state directory"))?;
    // Serialize independent GUI instances sharing this project's report store.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(state_path.with_extension("lock"))?;
    lock.lock_exclusive()?;
    let mut state: CheckState = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default();
    if state
        .last_checked_at
        .is_some_and(|last| now - last < Duration::days(CHECK_INTERVAL_DAYS))
    {
        return Ok(WeeklyCheckOutcome::NotDue);
    }
    let cache = models_cache_path().and_then(|path| rows_from_cache_file(&path));
    let snapshot = snapshot_rows();
    let effective = cache.unwrap_or_else(|| snapshot.clone());
    let diff = diff_catalog(&effective, &snapshot, saved_models);
    let outcome = if diff.is_empty() {
        WeeklyCheckOutcome::NoDiff
    } else {
        let body = diff.report_body(&effective);
        // The durable report itself is the deduplication record, even after a
        // crash between posting it and saving the last-check timestamp.
        if gwt_core::pm_report::load_reports(repo_path)?
            .iter()
            .any(|report| {
                report.kind == gwt_core::pm_report::PmReportKind::Progress && report.body == body
            })
        {
            WeeklyCheckOutcome::AlreadyReported
        } else {
            gwt_core::pm_report::post_report(
                repo_path,
                gwt_core::pm_report::PmReportKind::Progress,
                &body,
                None,
            )?;
            WeeklyCheckOutcome::Reported
        }
    };
    state.last_checked_at = Some(now);
    let encoded = serde_json::to_vec_pretty(&state)
        .map_err(|error| gwt_core::GwtError::Config(error.to_string()))?;
    crate::issue_monitor::durable_atomic_write(&state_path, &encoded)?;
    Ok(outcome)
}

/// Models of the saved Codex launch profiles in an Issue Monitor prefs file.
pub fn saved_codex_profile_models(prefs_path: &Path) -> Vec<String> {
    let Ok(prefs) = crate::load_issue_monitor_prefs(prefs_path) else {
        return Vec::new();
    };
    prefs
        .launch_profile_pool()
        .into_iter()
        .chain(prefs.launch_tiers.into_iter().flatten())
        .filter(|profile| {
            gwt_agent::resolve_agent_id(&profile.agent_id) == Some(gwt_agent::AgentId::Codex)
        })
        .filter_map(|profile| profile.model)
        .collect()
}

#[cfg(test)]
pub(crate) mod test_override {
    use std::{cell::RefCell, path::PathBuf};

    thread_local! {
        static PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Tests never read the developer's real Codex cache; they opt in with
    /// [`set`] and otherwise see the bundled snapshot.
    pub(crate) fn path() -> Option<PathBuf> {
        PATH.with(|path| path.borrow().clone())
    }

    pub(crate) struct Guard;

    pub(crate) fn set(path: PathBuf) -> Guard {
        PATH.with(|slot| *slot.borrow_mut() = Some(path));
        Guard
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            PATH.with(|slot| *slot.borrow_mut() = None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_json(rows: &[(&str, &str, &str, i64)]) -> String {
        let models: Vec<_> = rows
            .iter()
            .map(|(slug, name, visibility, priority)| {
                serde_json::json!({
                    "slug": slug, "display_name": name, "description": format!("{slug} desc"),
                    "default_reasoning_level": "medium",
                    "supported_reasoning_levels": [
                        {"effort": "low", "description": "l"},
                        {"effort": "medium", "description": "m"},
                        {"effort": "high", "description": "h"}
                    ],
                    "visibility": visibility, "priority": priority
                })
            })
            .collect();
        serde_json::json!({"fetched_at": "2026-11-09T00:00:00Z", "models": models}).to_string()
    }

    fn slugs(rows: &[CodexModelRow]) -> Vec<&str> {
        rows.iter().map(|row| row.slug.as_str()).collect()
    }

    #[test]
    fn cache_rows_keep_only_listed_models_in_priority_order() {
        let body = cache_json(&[
            ("b", "B Name", "list", 3),
            ("hidden", "Hidden", "hide", 1),
            ("a", "A Name", "list", 2),
        ]);
        let rows = parse_models_cache(&body).expect("valid cache");
        assert_eq!(slugs(&rows), ["a", "b"]);
        assert_eq!(rows[0].display_name, "A Name");
        assert_eq!(rows[0].efforts, ["low", "medium", "high"]);
        assert_eq!(rows[0].default_effort, "medium");
    }

    #[test]
    fn unusable_cache_bodies_yield_none() {
        assert!(parse_models_cache("not json").is_none());
        assert!(parse_models_cache(r#"{"models": "nope"}"#).is_none());
        assert!(parse_models_cache(r#"{"models":[{"slug":"gpt-6.1-sol"}]}"#).is_none());
    }

    #[test]
    fn valid_cache_with_only_hidden_models_stays_empty() {
        assert_eq!(
            parse_models_cache(&cache_json(&[("x", "X", "hide", 1)])),
            Some(Vec::new())
        );
    }

    #[test]
    fn snapshot_includes_terra_and_drops_gpt_5_5() {
        let rows = snapshot_rows();
        assert_eq!(
            slugs(&rows),
            [
                "gpt-6.1-sol",
                "gpt-6-astra",
                "gpt-6.1-terra",
                "gpt-6-sol",
                "gpt-6-luna",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
            ]
        );
    }

    #[test]
    fn current_rows_follow_cache_and_fall_back_to_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("models_cache.json");
        let _guard = test_override::set(path.clone());
        assert_eq!(*current_rows(), snapshot_rows(), "missing cache");
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(*current_rows(), snapshot_rows(), "malformed cache");
        std::fs::write(&path, r#"{"models":[{"slug":"gpt-6.1-sol"}]}"#).unwrap();
        assert_eq!(*current_rows(), snapshot_rows(), "missing visibility");
        std::fs::write(&path, cache_json(&[("only", "Only", "list", 1)])).unwrap();
        assert_eq!(slugs(&current_rows()), ["only"]);
    }

    fn rows(slugs: &[&str]) -> Vec<CodexModelRow> {
        slugs
            .iter()
            .map(|slug| CodexModelRow {
                slug: slug.to_string(),
                display_name: slug.to_string(),
                description: String::new(),
                default_effort: "medium".into(),
                efforts: vec!["medium".into()],
            })
            .collect()
    }

    #[test]
    fn diff_detects_added_removed_order_and_missing_saved_models() {
        let snapshot = rows(&["a", "b", "c"]);
        assert!(diff_catalog(&snapshot, &snapshot, &["a".into()]).is_empty());

        let diff = diff_catalog(&rows(&["a", "c", "b", "d"]), &snapshot, &[]);
        assert_eq!(diff.added, ["d"]);
        assert!(diff.removed.is_empty());
        assert!(diff.order_changed);

        let diff = diff_catalog(&rows(&["a", "b"]), &snapshot, &["c".into(), " ".into()]);
        assert_eq!(diff.removed, ["c"]);
        assert!(!diff.order_changed);
        assert_eq!(diff.missing_saved_models, ["c"]);
    }

    #[test]
    fn saved_models_include_codex_profiles_from_launch_tiers() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("issue-monitor.json");
        let profile = |agent: &str, model: &str| {
            serde_json::from_value(serde_json::json!({"agent_id": agent, "model": model})).unwrap()
        };
        let prefs = crate::IssueMonitorPrefs {
            launch_profiles: vec![profile("codex", "pool-model")],
            launch_tiers: vec![vec![
                profile("codex", "tier-model"),
                profile("claude", "opus"),
            ]],
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(
            saved_codex_profile_models(&path),
            ["pool-model", "tier-model"]
        );
    }

    #[test]
    fn weekly_check_reports_once_per_distinct_diff() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let cache = temp.path().join("models_cache.json");
        let _guard = test_override::set(cache.clone());
        let reports = || gwt_core::pm_report::load_reports(&repo).unwrap().len();
        let t0 = Utc::now();

        // Missing cache + no stale profile: snapshot vs snapshot, nothing to say.
        assert_eq!(
            run_weekly_check(&repo, &[], t0).unwrap(),
            WeeklyCheckOutcome::NoDiff
        );
        assert_eq!(
            run_weekly_check(&repo, &[], t0 + Duration::days(6)).unwrap(),
            WeeklyCheckOutcome::NotDue
        );

        let mut snapshot: Vec<_> = SNAPSHOT
            .iter()
            .enumerate()
            .map(|(index, row)| (row.0, row.1, "list", index as i64))
            .collect();
        snapshot.push(("gpt-new", "GPT New", "list", 99));
        snapshot.push(("gpt-reserve", "GPT-Reserve", "hide", 0));
        std::fs::write(&cache, cache_json(&snapshot)).unwrap();
        let t1 = t0 + Duration::days(7);
        assert_eq!(
            run_weekly_check(&repo, &["gpt-5.5".into()], t1).unwrap(),
            WeeklyCheckOutcome::Reported
        );
        assert_eq!(reports(), 1);
        let body = &gwt_core::pm_report::load_reports(&repo).unwrap()[0].body;
        assert!(body.contains("gpt-new"), "{body}");
        assert!(body.contains("gpt-5.5"), "{body}");
        assert!(!body.contains("gpt-reserve"), "{body}");

        let t2 = t1 + Duration::days(7);
        assert_eq!(
            run_weekly_check(&repo, &["gpt-5.5".into()], t2).unwrap(),
            WeeklyCheckOutcome::AlreadyReported
        );
        assert_eq!(reports(), 1, "the same diff is not reported again");

        snapshot.swap(0, 1);
        for (priority, row) in snapshot.iter_mut().enumerate() {
            row.3 = priority as i64;
        }
        std::fs::write(&cache, cache_json(&snapshot)).unwrap();
        let t3 = t2 + Duration::days(7);
        assert_eq!(
            run_weekly_check(&repo, &["gpt-5.5".into()], t3).unwrap(),
            WeeklyCheckOutcome::Reported
        );
        snapshot.swap(1, 2);
        for (priority, row) in snapshot.iter_mut().enumerate() {
            row.3 = priority as i64;
        }
        std::fs::write(&cache, cache_json(&snapshot)).unwrap();
        assert_eq!(
            run_weekly_check(&repo, &["gpt-5.5".into()], t3 + Duration::days(7)).unwrap(),
            WeeklyCheckOutcome::Reported,
            "a different ordering is a new difference even when the model set is unchanged"
        );
        assert_eq!(reports(), 3);
    }

    #[test]
    fn weekly_check_recovers_report_deduplication_after_state_loss() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let cache = temp.path().join("models_cache.json");
        let _guard = test_override::set(cache.clone());
        let now = Utc::now();
        let saved = ["retired-model".to_string()];
        assert_eq!(
            run_weekly_check(&repo, &saved, now).unwrap(),
            WeeklyCheckOutcome::Reported
        );
        std::fs::remove_file(check_state_path(&repo)).unwrap();
        assert_eq!(
            run_weekly_check(&repo, &saved, now).unwrap(),
            WeeklyCheckOutcome::AlreadyReported
        );
        assert_eq!(gwt_core::pm_report::load_reports(&repo).unwrap().len(), 1);

        let snapshot: Vec<_> = SNAPSHOT
            .iter()
            .enumerate()
            .map(|(priority, row)| (row.0, row.1, "list", priority as i64))
            .collect();
        std::fs::write(&cache, cache_json(&snapshot)).unwrap();
        assert_eq!(
            run_weekly_check(&repo, &saved, now + Duration::days(7)).unwrap(),
            WeeklyCheckOutcome::AlreadyReported,
            "cache recovery alone must not repeat the same model/profile difference"
        );
        assert_eq!(gwt_core::pm_report::load_reports(&repo).unwrap().len(), 1);
    }
}
