//! Current PM explanation and independently observed project facts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

mod reports;
mod tasks;
pub use reports::PmReportsReader;
pub use tasks::{task_detail, PmTaskDetail, PmTaskOverview};

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
        source: serde_json::to_value(status.source)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_tasks: Option<PmTaskOverview>,
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
    snapshot_from_inventory(repo, update, observed_at, &inventory)
}

fn snapshot_from_inventory(
    repo: &Path,
    update: Option<&gwt_core::update::UpdateState>,
    observed_at: DateTime<Utc>,
    inventory: &crate::session_inventory::SessionInventory,
) -> Result<PmCurrentSnapshot, String> {
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
        request_tasks: Some(tasks::load(repo)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_task_overview_uses_reports_versions_and_bounds_private_tracking_data() {
        use gwt_core::pm_request_tasks::{
            Action, Actor, ActorKind, IntakeSource, Request, TaskDraft, TaskStore,
        };
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let mut reader = PmReportsReader::default();
        let (first, absent) = reader.poll(&repo, None, None).unwrap();
        assert_eq!(
            serde_json::to_value(absent.current).unwrap()["request_tasks"]["state"],
            "absent"
        );
        let store = TaskStore::for_repo(&repo);
        let apply = |revision, kind, action| {
            store
                .apply(
                    &Actor {
                        kind,
                        id: "fixture".into(),
                    },
                    &Request {
                        operation_id: format!("op-{revision}"),
                        expected_revision: revision,
                        reason: "overview fixture".into(),
                        action,
                    },
                )
                .unwrap()
        };
        apply(
            0,
            ActorKind::Intake,
            Action::Capture {
                source: IntakeSource {
                    id: "private-source".into(),
                    project_key: project_dir(&repo)
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    original: "PRIVATE ORIGINAL MUST NOT CROSS THIS PROJECTION".into(),
                    authority: gwt_core::user_instruction::InstructionAuthority {
                        session_id: "synthetic-intake".into(),
                        turn_id: "synthetic-turn".into(),
                        prompt_sha256: gwt_core::pm_authorization::digest(
                            b"PRIVATE ORIGINAL MUST NOT CROSS THIS PROJECTION",
                        ),
                    },
                },
            },
        );
        let (captured, pending) = reader.poll(&repo, None, Some(&first)).unwrap();
        assert_eq!(
            captured.revision,
            first.revision + 1,
            "Task-only writes advance Reports"
        );
        assert_eq!(
            serde_json::to_value(pending.current).unwrap()["request_tasks"]["pending_intake"],
            1
        );
        let draft = |id: String| TaskDraft {
            title: "界".repeat(300),
            id,
            purpose: "目的".repeat(150),
            completion_conditions: (0..6)
                .map(|i| format!("{i}:{}", "条件".repeat(150)))
                .collect(),
            issue_numbers: vec![],
            goal_ids: vec![],
            assignee: Some("PM".into()),
        };
        apply(
            1,
            ActorKind::Pm,
            Action::Decompose {
                source_id: "private-source".into(),
                root: draft("private-000-root".into()),
                children: (0..55)
                    .map(|i| draft(format!("private-child-{i:03}")))
                    .collect(),
            },
        );
        apply(
            2,
            ActorKind::User,
            Action::Cancel {
                task_id: "private-child-000".into(),
            },
        );
        apply(
            3,
            ActorKind::Pm,
            Action::Complete {
                task_id: "private-child-054".into(),
                condition_revision: 1,
                review_generation: 1,
                evidence: vec!["synthetic:verified-child".into()],
            },
        );
        apply(
            4,
            ActorKind::Pm,
            Action::AddChild {
                parent_id: "private-child-001".into(),
                child: draft("private-grandchild".into()),
            },
        );
        let (updated, data) = reader.poll(&repo, None, Some(&captured)).unwrap();
        let value = serde_json::to_value(data.current).unwrap();
        let overview = &value["request_tasks"];
        assert_eq!(overview["state"], "ready");
        assert_eq!(overview["total"], 57);
        assert_eq!(overview["pending_intake"], 0);
        let rows = overview["tasks"].as_array().unwrap();
        assert_eq!(rows.len(), 50);
        let beyond_id_limit = format!("{:x}", Sha256::digest(b"private-child-049"));
        assert!(rows.iter().any(|row| row["key"] == beyond_id_limit));
        assert!(rows.iter().all(|row| row["status"] != "cancelled"));
        assert_eq!(rows[0]["key"].as_str().unwrap().len(), 64);
        let source_key = format!("{:x}", Sha256::digest(b"private-source"));
        assert!(rows.iter().all(|row| row["source_key"] == source_key));
        assert!(rows[0]["title"].as_str().unwrap().chars().count() <= 241);
        assert!(rows[1]["parent_key"].as_str().is_some());
        assert_eq!(
            rows[0]["status"], "pending",
            "child completion is not parent completion"
        );
        assert_eq!(
            rows[0]["child_progress"],
            serde_json::json!({
                "total": 55, "completed": 1, "cancelled": 1,
            }),
            "counts include hidden children and exclude the grandchild"
        );
        assert_eq!(
            rows[1]["child_progress"],
            serde_json::json!({
                "total": 1, "completed": 0, "cancelled": 0,
            })
        );
        assert!(
            rows[2].get("child_progress").is_none(),
            "leaves have no child progress"
        );
        let preview = &rows[0]["criteria_preview"];
        assert_eq!(preview["purpose"], format!("{}…", "目的".repeat(120)));
        assert_eq!(preview["completion_condition_total"], 6);
        let conditions = preview["completion_conditions"].as_array().unwrap();
        assert_eq!(conditions.len(), 5);
        for (index, condition) in conditions.iter().enumerate() {
            assert_eq!(
                condition,
                &serde_json::json!(format!("{index}:{}…", "条件".repeat(119)))
            );
        }
        let mut legacy = overview.clone();
        for row in legacy["tasks"].as_array_mut().unwrap() {
            row.as_object_mut().unwrap().remove("source_key");
            row.as_object_mut().unwrap().remove("criteria_preview");
            row.as_object_mut().unwrap().remove("child_progress");
        }
        let legacy: PmTaskOverview = serde_json::from_value(legacy).unwrap();
        let legacy = serde_json::to_value(legacy).unwrap();
        assert!(legacy["tasks"][0].get("source_key").is_none());
        assert!(legacy["tasks"][0].get("criteria_preview").is_none());
        assert!(legacy["tasks"][0].get("child_progress").is_none());
        let text = overview.to_string();
        for private in [
            "private-000-root",
            "private-child",
            "private-source",
            "PRIVATE ORIGINAL",
            "5:条件",
            "authority",
            "receipt",
        ] {
            assert!(
                !text.contains(private),
                "private tracking field leaked: {private}"
            );
        }
        assert!(
            reader.poll(&repo, None, Some(&updated)).is_none(),
            "unchanged Task data is acknowledged"
        );
        // PM status and request tracking are independent durable sources.
        std::fs::write(status_path(&repo), "broken PM status").unwrap();
        let (unrelated_error, data) = reader.poll(&repo, None, Some(&updated)).unwrap();
        assert!(data.error.is_none() && data.current_error.is_some());
        assert_eq!(
            serde_json::to_value(data.current).unwrap()["request_tasks"]["state"],
            "ready"
        );
        apply(
            5,
            ActorKind::Pm,
            Action::Progress {
                task_id: "private-child-001".into(),
                status: gwt_core::pm_request_tasks::TaskStatus::Waiting,
                wait_reason: Some("CI verdict".into()),
                next_action: "Review the checks".into(),
            },
        );
        let (progressed, data) = reader.poll(&repo, None, Some(&unrelated_error)).unwrap();
        assert_eq!(progressed.revision, unrelated_error.revision + 1);
        assert_eq!(
            serde_json::to_value(data.current).unwrap()["request_tasks"]["tasks"][1]["wait_reason"],
            "CI verdict"
        );
        std::fs::remove_file(status_path(&repo)).unwrap();
        let (updated, _) = reader.poll(&repo, None, Some(&progressed)).unwrap();
        let state = project_dir(&repo).join("project-state/request-tasks/state.json");
        std::fs::write(&state, "broken").unwrap();
        let (failed, data) = reader.poll(&repo, None, Some(&updated)).unwrap();
        assert_eq!(
            serde_json::to_value(data.current).unwrap()["request_tasks"]["state"],
            "error"
        );
        std::fs::remove_file(state).unwrap();
        assert!(
            reader.poll(&repo, None, Some(&failed)).is_none(),
            "the same safe error does not manufacture a new visible revision"
        );
        let (_, data) = reader.poll(&repo, None, None).unwrap();
        assert_eq!(
            serde_json::to_value(data.current).unwrap()["request_tasks"]["state"],
            "error",
            "initialized missing data is not empty"
        );
    }

    #[test]
    fn reports_poll_skips_acknowledged_loads_but_hydrates_independent_cursors() {
        use super::reports::{PmReportsData, PmReportsReader};
        let mut reader = PmReportsReader::default();
        let now = Utc::now();
        let data = PmReportsData::default();
        let (first, _) = reader
            .poll_with(Some(vec![]), None, now, || data.clone())
            .unwrap();
        assert!(reader
            .poll_with(Some(vec![]), Some(&first), now, || panic!(
                "unchanged inputs must not be read"
            ))
            .is_none());
        let (second, _) = reader
            .poll_with(Some(vec![]), None, now, || {
                panic!("new viewers reuse the snapshot")
            })
            .unwrap();
        assert_eq!(second, first);
        let mut changed = data.clone();
        changed.error = Some("unreadable status".into());
        let changed_input = Some(vec![super::reports::SourceStamp::Update(
            serde_json::json!(1),
        )]);
        let (failed, _) = reader
            .poll_with(changed_input.clone(), Some(&first), now, || changed.clone())
            .unwrap();
        assert_eq!(failed.revision, first.revision + 1);
        let (recovered, _) = reader
            .poll_with(changed_input.clone(), Some(&failed), now, || data.clone())
            .unwrap();
        assert_eq!(recovered.revision, failed.revision + 1);
        assert!(reader
            .poll_with(changed_input.clone(), Some(&recovered), now, || panic!(
                "the acknowledged recovery stays silent"
            ))
            .is_none());
        assert_eq!(
            reader
                .poll_with(changed_input, Some(&second), now, || panic!(
                    "another client reuses the recovered snapshot"
                ))
                .unwrap()
                .0,
            recovered
        );
        let task_input = Some(vec![super::reports::SourceStamp::Update(
            serde_json::json!(2),
        )]);
        changed.error = None;
        changed.current = Some(PmCurrentSnapshot {
            observed_at: now,
            status: None,
            stale_minutes: None,
            facts: PmMonitorFacts::default(),
            request_tasks: Some(PmTaskOverview::Error {
                message: "temporarily unreadable tasks".into(),
            }),
        });
        let (failed, _) = reader
            .poll_with(task_input.clone(), Some(&recovered), now, || {
                changed.clone()
            })
            .unwrap();
        assert!(reader
            .poll_with(task_input.clone(), Some(&failed), now, || panic!(
                "viewers share the task retry deadline"
            ))
            .is_none());
        assert!(reader
            .poll_with(
                task_input.clone(),
                Some(&failed),
                now + chrono::Duration::seconds(5),
                || changed,
            )
            .is_none());
        let retry_at = now + chrono::Duration::seconds(10);
        let (recovered, _) = reader
            .poll_with(task_input.clone(), Some(&failed), retry_at, || data)
            .expect("task reads recover on the shared retry deadline without a stamp change");
        assert_eq!(recovered.revision, failed.revision + 1);
        assert!(reader
            .poll_with(task_input, Some(&recovered), retry_at, || panic!(
                "the acknowledged task recovery stays silent"
            ))
            .is_none());
    }

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
        let mut prefs = crate::IssueMonitorPrefs {
            priority_order: vec![5349, 5342],
            ..Default::default()
        };
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
