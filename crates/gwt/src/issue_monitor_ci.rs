//! One-shot CI retry targets for the autonomous delivery gate.
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiRerunTarget {
    pub pr_number: u64,
    pub head_sha: String,
    pub run_id: u64,
}

impl CiRerunTarget {
    pub fn effect_id(&self) -> String {
        format!(
            "ci-rerun:{}:{}:{}",
            self.pr_number, self.head_sha, self.run_id
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiRerunOutcome {
    Requested,
    Observed,
    Refused(String),
    Unknown(String),
}

/// Select repository-owned Actions runs, deduplicating required jobs in one run.
pub fn failed_required_targets(
    rollup: &str,
    required: &[String],
    repo_slug: &str,
    pr_number: u64,
    head_sha: &str,
) -> Vec<CiRerunTarget> {
    let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(rollup) else {
        return Vec::new();
    };
    let runs: std::collections::BTreeSet<u64> = items
        .iter()
        .filter_map(|item| {
            let name = item["name"].as_str()?;
            if !required.iter().any(|required| required == name)
                || item["status"].as_str()? != "COMPLETED"
                || !matches!(
                    item["conclusion"].as_str()?,
                    "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE"
                )
            {
                return None;
            }
            repository_run_id(item["detailsUrl"].as_str()?, repo_slug)
        })
        .collect();
    runs.into_iter()
        .map(|run_id| CiRerunTarget {
            pr_number,
            head_sha: head_sha.to_string(),
            run_id,
        })
        .collect()
}

fn repository_run_id(url: &str, repo_slug: &str) -> Option<u64> {
    let prefix = format!("https://github.com/{repo_slug}/actions/runs/");
    let parts: Vec<_> = url.strip_prefix(&prefix)?.split('/').collect();
    if parts.len() != 3 || parts[1] != "job" || parts[2].parse::<u64>().ok()? == 0 {
        return None;
    }
    parts[0].parse::<u64>().ok().filter(|id| *id > 0)
}

fn gh(repo_path: &Path, args: &[&str]) -> Result<String, String> {
    let output = gwt_core::process_console::spawn_logged_blocking(
        &gwt_core::process_console::global(),
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        args,
        gwt_core::process_console::SpawnOptions::new("issue monitor CI rerun")
            .current_dir(repo_path)
            .forward_output(false),
    )
    .map_err(|error| error.to_string())?;
    if output.success() {
        Ok(output.stdout)
    } else {
        Err(output.stderr)
    }
}

fn run_snapshot(
    slug: &str,
    target: &CiRerunTarget,
    run: &mut impl FnMut(&[&str]) -> Result<String, String>,
) -> Result<Option<serde_json::Value>, String> {
    let payload = run(&[
        "api",
        &format!("/repos/{slug}/actions/runs/{}", target.run_id),
    ])?;
    let snapshot: serde_json::Value =
        serde_json::from_str(&payload).map_err(|error| error.to_string())?;
    if snapshot["id"].as_u64() != Some(target.run_id)
        || snapshot["repository"]["full_name"].as_str() != Some(slug)
        || snapshot["head_sha"].as_str() != Some(target.head_sha.as_str())
        || snapshot["event"].as_str() != Some("pull_request")
    {
        return Ok(None);
    }
    if snapshot["run_attempt"]
        .as_u64()
        .is_none_or(|attempt| attempt == 0)
    {
        return Err("CI run attempt is unknown".into());
    }
    Ok(Some(snapshot))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiRetryState {
    Ready,
    Pending,
    Failed,
}

/// Read the workflow attempt before reserving or reconsidering a submission.
pub fn retry_state(
    repo_path: &Path,
    target: &CiRerunTarget,
    consumed: bool,
) -> Result<CiRetryState, String> {
    let (owner, repo) = crate::issue_monitor_worker::github_remote_owner_and_repo(repo_path)
        .map_err(|error| error.to_string())?;
    retry_state_with(&format!("{owner}/{repo}"), target, consumed, |args| {
        gh(repo_path, args)
    })
}

fn retry_state_with(
    slug: &str,
    target: &CiRerunTarget,
    consumed: bool,
    mut run: impl FnMut(&[&str]) -> Result<String, String>,
) -> Result<CiRetryState, String> {
    let Some(snapshot) = run_snapshot(slug, target, &mut run)? else {
        // A proven ineligible workflow retains normal failure remediation.
        return Ok(CiRetryState::Failed);
    };
    let first = snapshot["run_attempt"].as_u64() == Some(1);
    if first && consumed {
        return Err(format!("CI rerun {} has consumed its one submission; awaiting GitHub attempt readback, no duplicate will be sent", target.run_id));
    }
    match snapshot["status"].as_str() {
        Some("queued" | "in_progress" | "waiting" | "pending" | "requested") => {
            Ok(CiRetryState::Pending)
        }
        Some("completed") => match snapshot["conclusion"].as_str() {
            Some("success" | "neutral" | "skipped") => Ok(CiRetryState::Pending),
            Some("failure" | "timed_out" | "startup_failure") if first => Ok(CiRetryState::Ready),
            Some(
                "failure" | "timed_out" | "startup_failure" | "cancelled" | "action_required"
                | "stale",
            ) => Ok(CiRetryState::Failed),
            _ => Err("CI retry conclusion is unknown".into()),
        },
        _ => Err("CI retry status is unknown".into()),
    }
}

pub fn execute_ci_rerun(
    repo_path: &Path,
    target: &CiRerunTarget,
    allow_submit: bool,
) -> CiRerunOutcome {
    let (owner, repo) = match crate::issue_monitor_worker::github_remote_owner_and_repo(repo_path) {
        Ok(repo) => repo,
        Err(error) => return CiRerunOutcome::Refused(error.to_string()),
    };
    execute_with(&format!("{owner}/{repo}"), target, allow_submit, |args| {
        gh(repo_path, args)
    })
}

fn execute_with(
    slug: &str,
    target: &CiRerunTarget,
    allow_submit: bool,
    mut run: impl FnMut(&[&str]) -> Result<String, String>,
) -> CiRerunOutcome {
    let snapshot = (|| {
        let payload = run(&[
            "pr",
            "view",
            &target.pr_number.to_string(),
            "--repo",
            slug,
            "--json",
            "state,headRefOid,statusCheckRollup",
        ])?;
        let pr: serde_json::Value =
            serde_json::from_str(&payload).map_err(|error| error.to_string())?;
        if pr["state"].as_str() != Some("OPEN")
            || pr["headRefOid"].as_str() != Some(target.head_sha.as_str())
        {
            return Err("CI rerun refused: PR is closed or its head changed".to_string());
        }
        let belongs_to_pr = pr["statusCheckRollup"].as_array().is_some_and(|checks| {
            checks.iter().any(|check| {
                check["detailsUrl"]
                    .as_str()
                    .and_then(|url| repository_run_id(url, slug))
                    == Some(target.run_id)
            })
        });
        if !belongs_to_pr {
            return Err("CI rerun refused: run is absent from the current PR checks".into());
        }
        run_snapshot(slug, target, &mut run)
    })();
    let snapshot = match snapshot {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            return CiRerunOutcome::Refused(
                "CI run repository, identity, head or event is ineligible".into(),
            )
        }
        Err(error) => return CiRerunOutcome::Refused(error),
    };
    if snapshot["run_attempt"]
        .as_u64()
        .is_some_and(|attempt| attempt > 1)
    {
        return CiRerunOutcome::Observed;
    }
    if !allow_submit {
        return CiRerunOutcome::Unknown(
            "CI rerun submission was already consumed; recovered effects only read back".into(),
        );
    }
    if snapshot["status"].as_str() != Some("completed")
        || !matches!(
            snapshot["conclusion"].as_str(),
            Some("failure" | "timed_out" | "startup_failure")
        )
    {
        return CiRerunOutcome::Refused("CI run is not a completed failure".into());
    }
    match run(&[
        "api",
        "--method",
        "POST",
        &format!(
            "/repos/{slug}/actions/runs/{}/rerun-failed-jobs",
            target.run_id
        ),
    ]) {
        Ok(_) => CiRerunOutcome::Requested,
        Err(error) => {
            CiRerunOutcome::Unknown(format!("CI rerun submission outcome is unknown: {error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn target() -> CiRerunTarget {
        CiRerunTarget {
            pr_number: 7,
            head_sha: "abc123".into(),
            run_id: 90,
        }
    }

    fn pr() -> String {
        json!({"state":"OPEN","headRefOid":"abc123","statusCheckRollup":[
            {"detailsUrl":"https://github.com/owner/repo/actions/runs/90/job/10"}
        ]})
        .to_string()
    }

    fn workflow(attempt: u64) -> String {
        json!({"id":90,"repository":{"full_name":"owner/repo"},"head_sha":"abc123",
            "event":"pull_request","status":"completed","conclusion":"failure","run_attempt":attempt}).to_string()
    }

    #[test]
    fn ci_rerun_selects_only_failed_required_repository_runs_once() {
        let check = |name, url| json!({"name":name,"status":"COMPLETED","conclusion":"FAILURE","detailsUrl":url});
        let rollup = json!([
            check(
                "Rust",
                "https://github.com/owner/repo/actions/runs/90/job/10"
            ),
            check(
                "Windows",
                "https://github.com/owner/repo/actions/runs/90/job/11"
            ),
            check(
                "Optional",
                "https://github.com/owner/repo/actions/runs/91/job/12"
            ),
            check(
                "Foreign",
                "https://github.com/other/repo/actions/runs/92/job/13"
            ),
            check("Malformed", "https://example.com/actions/runs/93/job/14")
        ])
        .to_string();
        let required = ["Rust", "Windows", "Foreign", "Malformed"].map(str::to_string);
        assert_eq!(
            failed_required_targets(&rollup, &required, "owner/repo", 7, "abc123"),
            vec![target()]
        );
    }

    #[test]
    fn ci_rerun_submits_failed_jobs_only_after_pr_and_run_proof() {
        let mut calls = Vec::new();
        let result = execute_with("owner/repo", &target(), true, |args| {
            calls.push(args.join(" "));
            if args[0] == "pr" {
                return Ok(pr());
            }
            if args.contains(&"POST") {
                return Ok(String::new());
            }
            Ok(workflow(1))
        });
        assert_eq!(result, CiRerunOutcome::Requested);
        assert_eq!(
            calls.last().unwrap(),
            "api --method POST /repos/owner/repo/actions/runs/90/rerun-failed-jobs"
        );
        assert_eq!(calls.len(), 3);
    }

    #[test]
    fn ci_rerun_never_posts_recovered_or_previously_retried_effects() {
        for (allow_submit, attempt) in [(false, 1), (true, 2)] {
            let result = execute_with("owner/repo", &target(), allow_submit, |args| {
                assert!(!args.contains(&"POST"));
                if args[0] == "pr" {
                    return Ok(pr());
                }
                Ok(workflow(attempt))
            });
            assert!(matches!(
                result,
                CiRerunOutcome::Observed | CiRerunOutcome::Unknown(_)
            ));
        }
    }
    #[test]
    fn ci_rerun_refuses_changed_identity_and_preserves_unknown_post() {
        let changed = execute_with("owner/repo", &target(), true, |args| {
            assert_eq!(args[0], "pr");
            Ok(json!({"state":"OPEN","headRefOid":"new-head"}).to_string())
        });
        assert!(matches!(changed, CiRerunOutcome::Refused(_)));
        let foreign = execute_with("owner/repo", &target(), true, |args| {
            assert!(!args.contains(&"POST"));
            if args[0] == "pr" {
                return Ok(pr());
            }
            Ok(workflow(1).replace("owner/repo", "foreign/repo"))
        });
        assert!(matches!(foreign, CiRerunOutcome::Refused(_)));
        let ambiguous = execute_with("owner/repo", &target(), true, |args| {
            if args[0] == "pr" {
                return Ok(pr());
            }
            if args.contains(&"POST") {
                return Err("connection lost".into());
            }
            Ok(workflow(1))
        });
        assert!(matches!(ambiguous, CiRerunOutcome::Unknown(_)));
    }

    #[test]
    fn ci_rerun_observation_waits_for_retry_and_returns_terminal_failure() {
        assert_eq!(
            retry_state_with("owner/repo", &target(), false, |_| Ok(
                workflow(1).replace("completed", "in_progress")
            )),
            Ok(CiRetryState::Pending)
        );
        assert_eq!(
            retry_state_with("owner/repo", &target(), false, |_| Ok(workflow(1))),
            Ok(CiRetryState::Ready)
        );
        assert!(retry_state_with("owner/repo", &target(), true, |_| Ok(workflow(1))).is_err());
        assert_eq!(
            retry_state_with("owner/repo", &target(), true, |_| Ok(
                workflow(2).replace("completed", "in_progress")
            )),
            Ok(CiRetryState::Pending)
        );
        assert_eq!(
            retry_state_with("owner/repo", &target(), true, |_| Ok(workflow(2))),
            Ok(CiRetryState::Failed)
        );
    }
    #[test]
    fn ci_rerun_refuses_run_absent_from_current_pr_checks() {
        let mut posted = false;
        let result = execute_with("owner/repo", &target(), true, |args| {
            if args[0] == "pr" {
                return Ok(
                    json!({"state":"OPEN","headRefOid":"abc123","statusCheckRollup":[
                        {"detailsUrl":"https://github.com/owner/repo/actions/runs/91/job/10"}
                    ]})
                    .to_string(),
                );
            }
            if args.contains(&"POST") {
                posted = true;
                return Ok(String::new());
            }
            Ok(workflow(1))
        });
        assert!(matches!(result, CiRerunOutcome::Refused(_)));
        assert!(!posted, "same head on another PR cannot authorize this run");
    }

    #[test]
    fn ci_rerun_ineligible_workflow_returns_to_remediation() {
        assert_eq!(
            retry_state_with("owner/repo", &target(), false, |_| Ok(
                workflow(1).replace("pull_request", "push")
            )),
            Ok(CiRetryState::Failed)
        );
    }
}
