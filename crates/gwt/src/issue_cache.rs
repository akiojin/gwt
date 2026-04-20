#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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

    sync_issue_cache_from_remote(repo_path, cache_root)
}

pub(crate) fn sync_issue_cache_from_remote(
    repo_path: &Path,
    cache_root: &Path,
) -> Result<(), String> {
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

fn gh_executable() -> std::ffi::OsString {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("GWT_TEST_GH") {
        return path;
    }
    std::ffi::OsString::from("gh")
}

fn fetch_issue_list_snapshots(repo_path: &Path) -> Result<Vec<IssueSnapshot>, String> {
    let output = Command::new(gh_executable())
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
    #[cfg(target_os = "windows")]
    use std::env;
    use std::fs;

    use super::*;
    use tempfile::tempdir;

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

    #[test]
    fn cache_entry_detection_and_detached_sync_short_circuit_non_repo_paths() {
        let temp = tempdir().expect("tempdir");
        let cache_root = temp.path().join("issues");
        fs::create_dir_all(cache_root.join("not-an-issue")).expect("create cache dir");
        assert!(!issue_cache_has_entries(&cache_root));

        fs::create_dir_all(cache_root.join("1234")).expect("create numeric issue dir");
        assert!(issue_cache_has_entries(&cache_root));

        let repo_path = temp.path().join("plain-dir");
        fs::create_dir_all(&repo_path).expect("create repo path");
        assert_eq!(
            issue_cache_root_for_repo_path_or_detached(&repo_path),
            detached_issue_cache_root()
        );
        assert!(sync_issue_cache_from_remote_if_missing(&repo_path, &cache_root).is_ok());
    }

    #[test]
    fn parse_issue_list_snapshots_defaults_missing_fields_and_closed_state() {
        let snapshots = parse_issue_list_snapshots(
            r#"[{
                "number": 12,
                "title": "Closed issue",
                "state": "closed",
                "labels": [{}]
            }]"#,
        )
        .expect("parse snapshots");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].number.0, 12);
        assert_eq!(snapshots[0].body, "");
        assert!(snapshots[0].labels.is_empty());
        assert_eq!(snapshots[0].state, IssueState::Closed);
        assert_eq!(
            snapshots[0].updated_at,
            UpdatedAt::new("1970-01-01T00:00:00Z")
        );

        let err = parse_issue_list_snapshots("{not-json").unwrap_err();
        assert!(!err.is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn sync_issue_cache_from_remote_writes_entries_and_surfaces_gh_failures() {
        let _guard = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempdir().expect("tempdir");
        let repo_path = temp.path().join("repo");
        let cache_root = temp.path().join("cache");
        let empty_cache_root = temp.path().join("empty-cache");
        fs::create_dir_all(&repo_path).expect("create repo path");
        let fake_gh = repo_path.join("gh.cmd");
        fs::write(
            &fake_gh,
            "@echo off\r\n\
if /I \"%FAKE_GH_MODE%\"==\"fail\" (\r\n\
  >&2 echo gh api down\r\n\
  exit /b 1\r\n\
)\r\n\
if /I \"%FAKE_GH_MODE%\"==\"empty\" (\r\n\
  echo []\r\n\
  exit /b 0\r\n\
)\r\n\
echo [{\"number\":7,\"title\":\"Cached issue\",\"body\":\"Body\",\"labels\":[{\"name\":\"gwt-spec\"}],\"state\":\"OPEN\",\"url\":\"https://example.test/issues/7\",\"updatedAt\":\"2026-04-20T00:00:00Z\"}]\r\n\
exit /b 0\r\n",
        )
        .expect("write fake gh");
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .status()
            .expect("git init");

        let old_gh = env::var_os("GWT_TEST_GH");
        env::set_var("GWT_TEST_GH", &fake_gh);

        env::set_var("FAKE_GH_MODE", "ok");
        sync_issue_cache_from_remote(&repo_path, &cache_root).expect("sync success");
        let cache = Cache::new(cache_root.clone());
        let entry = cache
            .load_entry(IssueNumber(7))
            .expect("cached issue entry should exist");
        assert_eq!(entry.snapshot.title, "Cached issue");
        assert_eq!(entry.snapshot.body, "Body");
        assert_eq!(entry.snapshot.labels, vec!["gwt-spec".to_string()]);

        env::set_var("FAKE_GH_MODE", "empty");
        sync_issue_cache_from_remote(&repo_path, &empty_cache_root).expect("empty sync succeeds");
        assert!(empty_cache_root.is_dir());
        assert!(fs::read_dir(&empty_cache_root)
            .expect("read empty cache")
            .next()
            .is_none());

        env::set_var("FAKE_GH_MODE", "fail");
        let err = sync_issue_cache_from_remote(&repo_path, &cache_root).unwrap_err();
        assert!(err.contains("gh issue list: gh api down"));

        match old_gh {
            Some(value) => env::set_var("GWT_TEST_GH", value),
            None => env::remove_var("GWT_TEST_GH"),
        }
        env::remove_var("FAKE_GH_MODE");
    }
}
