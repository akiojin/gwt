use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gwt_core::{
    paths::gwt_cache_dir,
    repo_hash::{compute_repo_hash, RepoHash},
};
use gwt_github::{client::IssueSnapshot, Cache, IssueNumber, IssueState, UpdatedAt};
use serde_json::Value;

const DETACHED_REPO_CACHE_DIR: &str = "__detached__";

pub(crate) fn issue_cache_base_root() -> PathBuf {
    gwt_cache_dir().join("issues")
}

pub(crate) fn detached_issue_cache_root() -> PathBuf {
    issue_cache_base_root().join(DETACHED_REPO_CACHE_DIR)
}

pub(crate) fn issue_cache_root_for_repo_hash(repo_hash: &RepoHash) -> PathBuf {
    issue_cache_base_root().join(repo_hash.as_str())
}

pub(crate) fn issue_cache_root_for_repo_slug(owner: &str, repo: &str) -> PathBuf {
    let remote = format!("https://github.com/{owner}/{repo}.git");
    issue_cache_root_for_repo_hash(&compute_repo_hash(&remote))
}

pub(crate) fn issue_cache_root_for_repo_path(repo_path: &Path) -> Option<PathBuf> {
    crate::index_worker::detect_repo_hash(repo_path)
        .map(|repo_hash| issue_cache_root_for_repo_hash(&repo_hash))
}

pub(crate) fn issue_cache_root_for_repo_path_or_detached(repo_path: &Path) -> PathBuf {
    issue_cache_root_for_repo_path(repo_path).unwrap_or_else(detached_issue_cache_root)
}

pub(crate) fn sync_issue_cache_from_remote_if_missing(
    repo_path: &Path,
    cache_root: &Path,
) -> Result<(), String> {
    if issue_cache_has_entries(cache_root) {
        return Ok(());
    }
    if issue_cache_root_for_repo_path(repo_path).is_none() {
        return Ok(());
    }

    let snapshots = fetch_issue_list_snapshots(repo_path)?;
    if snapshots.is_empty() {
        fs::create_dir_all(cache_root).map_err(|err| err.to_string())?;
        return Ok(());
    }

    let cache = Cache::new(cache_root.to_path_buf());
    for snapshot in &snapshots {
        cache
            .write_snapshot(snapshot)
            .map_err(|err| format!("write issue cache: {err}"))?;
    }
    Ok(())
}

fn issue_cache_has_entries(cache_root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(cache_root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false)
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.parse::<u64>().is_ok())
    })
}

fn fetch_issue_list_snapshots(repo_path: &Path) -> Result<Vec<IssueSnapshot>, String> {
    let output = Command::new("gh")
        .args([
            "issue",
            "list",
            "--state",
            "all",
            "--limit",
            "200",
            "--json",
            "number,title,body,labels,state,url,updatedAt",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|err| format!("gh issue list: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh issue list: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    parse_issue_list_snapshots(&String::from_utf8_lossy(&output.stdout))
}

fn parse_issue_list_snapshots(json: &str) -> Result<Vec<IssueSnapshot>, String> {
    let raw: Vec<Value> = serde_json::from_str(json).map_err(|err| err.to_string())?;
    Ok(raw
        .into_iter()
        .filter_map(|issue| {
            let number = issue.get("number")?.as_u64()?;
            let title = issue
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            let body = issue
                .get("body")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            let labels = issue
                .get("labels")
                .and_then(|value| value.as_array())
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(|label| {
                            label
                                .get("name")
                                .and_then(|value| value.as_str())
                                .map(str::to_string)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let state = match issue.get("state").and_then(|value| value.as_str()) {
                Some("CLOSED") | Some("closed") => IssueState::Closed,
                _ => IssueState::Open,
            };
            let updated_at = issue
                .get("updatedAt")
                .and_then(|value| value.as_str())
                .unwrap_or("1970-01-01T00:00:00Z")
                .to_string();

            Some(IssueSnapshot {
                number: IssueNumber(number),
                title,
                body,
                labels,
                state,
                updated_at: UpdatedAt::new(updated_at),
                comments: vec![],
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_slug_root_uses_repo_hash_subdirectory() {
        let root = issue_cache_root_for_repo_slug("example", "repo");
        let expected = compute_repo_hash("https://github.com/example/repo.git");
        assert!(root.ends_with(format!("issues/{}", expected.as_str())));
    }

    #[test]
    fn detached_root_uses_isolated_subdirectory() {
        assert!(detached_issue_cache_root().ends_with("issues/__detached__"));
    }

    #[test]
    fn parse_issue_list_snapshots_parses_list_payload() {
        let snapshots = parse_issue_list_snapshots(
            r#"[{
                "number": 1776,
                "title": "Launch Agent issue linkage",
                "body": "Body",
                "labels": [{"name": "ux"}],
                "state": "OPEN",
                "url": "https://github.com/example/repo/issues/1776",
                "updatedAt": "2026-04-13T00:00:00Z"
            }]"#,
        )
        .expect("parse snapshots");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].number.0, 1776);
        assert_eq!(snapshots[0].title, "Launch Agent issue linkage");
        assert_eq!(snapshots[0].body, "Body");
        assert_eq!(snapshots[0].labels, vec!["ux".to_string()]);
        assert_eq!(snapshots[0].updated_at.0, "2026-04-13T00:00:00Z");
    }
}
