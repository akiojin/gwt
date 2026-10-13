//! Current PM explanation and independently observed project facts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmIssueFact {
    pub issue_number: Option<u64>,
    pub title: Option<String>,
    pub last_activity_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmReleaseFact {
    pub number: u64,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmMonitorFacts {
    pub source: String,
    pub active_windows: Vec<PmIssueFact>,
    pub inventory_incomplete: bool,
    pub queue_head: Option<PmIssueFact>,
    pub needs_human_count: Option<usize>,
    pub release: Option<PmReleaseFact>,
    pub release_observed_at: Option<DateTime<Utc>>,
    pub pending_update: Option<String>,
    pub update_known: bool,
    pub error: Option<String>,
}

fn facts_from_monitor(
    status: &crate::IssueMonitorAgentStatus,
    windows: Vec<PmIssueFact>,
    title: impl Fn(u64) -> Option<String>,
) -> PmMonitorFacts {
    PmMonitorFacts {
        source: serde_json::to_value(&status.source)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unavailable".into()),
        active_windows: windows,
        queue_head: status.queue.first().map(|number| PmIssueFact {
            issue_number: Some(*number),
            title: title(*number),
            last_activity_at: None,
        }),
        needs_human_count: Some(status.needs_human.len()),
        ..Default::default()
    }
}

fn fingerprint_from_prefs(prefs: &crate::IssueMonitorPrefs) -> String {
    // Only decisions and lifecycle changes matter here. Polling, heartbeat and
    // activity clocks must never manufacture a new PM reporting obligation.
    let semantic = serde_json::json!({
        "enabled": prefs.enabled,
        "queue": prefs.priority_order,
        "launched": prefs.launched_issues,
        "launching": prefs.launching_issues.iter().map(|row| row.issue_number).collect::<Vec<_>>(),
        "failed": prefs.failed_issues.iter().map(|row| row.issue_number).collect::<Vec<_>>(),
        "merged": prefs.merged_issues,
        "phases": prefs.autonomous_records.iter().map(|row| (
            row.issue_number, &row.phase, &row.active_launch_id, &row.needs_human_kind, row.pr_number
        )).collect::<Vec<_>>(),
        "update_drain": prefs.update_drain.as_ref().map(|drain| (&drain.version, &drain.reason)),
    });
    format!("{:x}", Sha256::digest(semantic.to_string().as_bytes()))
}

fn release_from_cache(
    cache: &gwt_git::pr_status::PrInventoryCache,
) -> (Option<PmReleaseFact>, Option<DateTime<Utc>>) {
    let release = cache
        .rows
        .iter()
        .find(|row| {
            row["headRefName"] == gwt_git::release_status::DEFAULT_RELEASE_BRANCH
                && row["baseRefName"] == gwt_git::release_status::DEFAULT_BASE_BRANCH
        })
        .and_then(|row| {
            Some(PmReleaseFact {
                number: row["number"].as_u64()?,
                title: row["title"].as_str()?.to_owned(),
                url: row["url"].as_str()?.to_owned(),
            })
        });
    (release, cache.fetched_at)
}

fn project_dir(repo: &Path) -> PathBuf {
    repo.ancestors()
        .find_map(|ancestor| {
            crate::pm_registry::pm_worktree_store_dir(ancestor).or_else(|| {
                crate::pm_registry::is_canonical_pm_worktree(ancestor)
                    .then(|| ancestor.parent()?.parent().map(Path::to_path_buf))
                    .flatten()
            })
        })
        .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(repo))
}

/// PM checkout and nested cwd share their owning project's status ledger.
pub fn status_path(repo: &Path) -> PathBuf {
    project_dir(repo).join("project-state/pm-status.jsonl")
}

pub fn monitor_fingerprint(repo: &Path) -> Option<String> {
    let path = project_dir(repo).join("project-state/issue-monitor.json");
    if !path.exists() {
        return None;
    }
    let prefs = crate::load_issue_monitor_prefs(&path).ok()?;
    // issue.monitor.status folds Board handoffs into needs_human. Those can
    // change independently of the preferences; activity timestamps cannot.
    let escalations = gwt_core::coordination::load_escalation_store(repo).ok()?;
    let semantic = serde_json::json!([
        fingerprint_from_prefs(&prefs),
        escalations.open_owner_issue_numbers(),
    ]);
    Some(format!(
        "{:x}",
        Sha256::digest(semantic.to_string().as_bytes())
    ))
}

pub fn latest_status(
    repo: &Path,
) -> gwt_core::Result<Option<gwt_core::pm_status::PmCurrentStatus>> {
    gwt_core::pm_status::latest_status_from_path(&status_path(repo))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PmCurrentSnapshot {
    pub observed_at: DateTime<Utc>,
    pub status: Option<gwt_core::pm_status::PmCurrentStatus>,
    pub stale_minutes: Option<i64>,
    pub facts: PmMonitorFacts,
}

/// Runs off the GUI thread. All facts have their own authority; a missing
/// monitor or remote observation cannot erase the PM's authored explanation.
pub fn snapshot(
    repo: &Path,
    update: Option<&gwt_core::update::UpdateState>,
) -> Result<PmCurrentSnapshot, String> {
    let observed_at = Utc::now();
    let inventory =
        crate::session_inventory::observe_sessions(repo, &gwt_core::paths::gwt_sessions_dir());
    let monitor = crate::cli::issue::load_monitor_agent_status(repo, inventory.sessions.len()).map(
        |(mut status, _capacity)| {
            crate::cli::issue::merge_board_escalations_into_needs_human(repo, &mut status);
            status
        },
    );
    let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(repo);
    let title = |number| crate::issue_cache::load_issue_title_from_cache(&cache_root, number);
    let windows = inventory
        .sessions
        .iter()
        .map(|session| {
            let last_activity_at = monitor
                .as_ref()
                .ok()
                .and_then(|status| {
                    status
                        .inbox
                        .iter()
                        .find(|row| Some(row.issue_number) == session.issue_number)
                        .and_then(|row| row.last_activity_at.clone())
                })
                .or_else(|| {
                    gwt_agent::Session::load(
                        &gwt_core::paths::gwt_sessions_dir()
                            .join(format!("{}.toml", session.session_id)),
                    )
                    .ok()
                    .map(|row| row.last_activity_at.to_rfc3339())
                });
            PmIssueFact {
                issue_number: session.issue_number,
                title: session.issue_number.and_then(title),
                last_activity_at,
            }
        })
        .collect();
    let mut facts = match monitor {
        Ok(status) => facts_from_monitor(&status, windows, title),
        Err(error) => PmMonitorFacts {
            source: "unavailable".into(),
            active_windows: windows,
            error: Some(format!("Monitor facts unavailable: {error}")),
            ..Default::default()
        },
    };
    facts.inventory_incomplete = !inventory.uncertainties.is_empty();
    let directory = project_dir(repo);
    let cache_path = directory.join(gwt_git::PR_INVENTORY_CACHE_FILE);
    let settings =
        gwt_config::Settings::load_from_path(&gwt_core::paths::gwt_home().join("config.toml"))
            .unwrap_or_default();
    let options = gwt_git::PrInventoryOptions {
        cache_ttl_secs: settings.pr_inventory.cache_ttl_secs,
        checks_refresh_secs: settings.pr_inventory.checks_refresh_secs,
        ..Default::default()
    };
    // Reuse pr.list's budget/TTL read, without advancing its escalation history.
    // A failed refresh can still expose the previous observation and its time.
    let _ = gwt_git::pr_status::fetch_pr_inventory_cached(repo, &cache_path, &options);
    (facts.release, facts.release_observed_at) =
        release_from_cache(&gwt_git::pr_status::PrInventoryCache::load(&cache_path));
    if let Some(staged) = gwt_core::update::load_pending_update_manifest() {
        facts.pending_update = Some(staged.version);
        facts.update_known = true;
    } else if let Some(update) = update {
        match update {
            gwt_core::update::UpdateState::Available { latest, .. } => {
                facts.pending_update = Some(latest.clone());
                facts.update_known = true;
            }
            gwt_core::update::UpdateState::UpToDate { checked_at } => {
                facts.update_known = checked_at.is_some()
            }
            gwt_core::update::UpdateState::Failed { .. } => {}
        }
    }
    let status = latest_status(repo)
        .map_err(|error| format!("PM current status could not be loaded: {error}"))?;
    let stale_minutes = status
        .as_ref()
        .and_then(|status| status.stale_minutes(Utc::now()));
    Ok(PmCurrentSnapshot {
        observed_at,
        status,
        stale_minutes,
        facts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn status_store_uses_canonical_pm_cwd_below_a_symlinked_home() {
        let temp = tempfile::tempdir().unwrap();
        let actual = temp.path().join("actual-home");
        std::fs::create_dir(&actual).unwrap();
        let alias = temp.path().join("home-link");
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(&alias);
        let repo = actual.join("repo");
        std::fs::create_dir(&repo).unwrap();
        let project = gwt_core::paths::gwt_project_dir_for_repo_path(&repo);
        let pm = project.join("pm/worktree");
        std::fs::create_dir_all(pm.join("src/nested")).unwrap();
        let canonical = std::fs::canonicalize(&pm).unwrap();
        assert!(crate::pm_registry::pm_worktree_store_dir(&canonical).is_none());
        assert!(crate::pm_registry::is_canonical_pm_worktree(&canonical));
        let input = serde_json::from_value(serde_json::json!({
            "items":[{"kind":"waiting", "text":"Waiting for CI", "started_at":"2026-10-13T00:00:00Z"}],
            "next_trigger":"CI completes"
        })).unwrap();
        let saved = gwt_core::pm_status::set_status(&repo, input, None).unwrap();
        assert_eq!(latest_status(&canonical).unwrap(), Some(saved.clone()));
        assert_eq!(
            latest_status(&canonical.join("src/nested")).unwrap(),
            Some(saved)
        );
    }

    #[test]
    fn monitor_projection_keeps_live_windows_queue_and_needs_human_separate() {
        let mut status = crate::IssueMonitorState::new(Default::default()).agent_status();
        status.queue = vec![5342, 5341];
        status.needs_human = vec![20, 21];
        let windows = vec![PmIssueFact {
            issue_number: Some(5349),
            title: Some("Current PM".into()),
            last_activity_at: Some("2026-10-13T09:00:00Z".into()),
        }];
        let facts = facts_from_monitor(&status, windows, |number| Some(format!("Issue {number}")));
        assert_eq!(facts.active_windows[0].issue_number, Some(5349));
        assert_eq!(
            facts.active_windows[0].last_activity_at.as_deref(),
            Some("2026-10-13T09:00:00Z")
        );
        let head = facts.queue_head.unwrap();
        assert_eq!(head.issue_number, Some(5342));
        assert_eq!(head.title.as_deref(), Some("Issue 5342"));
        assert_eq!(facts.needs_human_count, Some(2));
    }

    #[test]
    fn semantic_monitor_fingerprint_ignores_scan_time_and_tracks_lifecycle_changes() {
        let mut prefs = crate::IssueMonitorPrefs::default();
        prefs.priority_order = vec![5349, 5342];
        let first = fingerprint_from_prefs(&prefs);
        prefs.last_scan_at = Some("2026-10-13T09:00:00Z".into());
        assert_eq!(fingerprint_from_prefs(&prefs), first);
        prefs.merged_issues.push(5349);
        assert_ne!(fingerprint_from_prefs(&prefs), first);
        let merged = fingerprint_from_prefs(&prefs);
        prefs.priority_order.reverse();
        assert_ne!(fingerprint_from_prefs(&prefs), merged);
    }

    #[test]
    fn release_projection_uses_observed_release_pr_and_does_not_invent_absence() {
        let mut cache = gwt_git::pr_status::PrInventoryCache::default();
        assert_eq!(release_from_cache(&cache), (None, None));
        let observed = "2026-10-13T09:00:00Z".parse().unwrap();
        cache.fetched_at = Some(observed);
        cache.rows = vec![
            serde_json::json!({"number": 5352, "title": "Release v9.117.2", "url": "https://github.com/akiojin/gwt/pull/5352", "headRefName": "develop", "baseRefName": "main"}),
            serde_json::json!({"number": 5340, "title": "Other feature", "headRefName": "work/issue-5340", "baseRefName": "develop"}),
        ];
        let (release, at) = release_from_cache(&cache);
        assert_eq!(release.unwrap().number, 5352);
        assert_eq!(at, Some(observed));
    }
}
