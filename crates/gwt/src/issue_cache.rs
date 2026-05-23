#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use gwt_core::{
    paths::gwt_cache_dir,
    repo_hash::{compute_repo_hash, RepoHash},
};
use gwt_github::{
    cache::write_atomic,
    client::{CommentId, CommentSnapshot, IssueSnapshot},
    Cache, IssueNumber, IssueState, UpdatedAt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const DETACHED_REPO_CACHE_DIR: &str = "__detached__";
const SPEC_LABEL: &str = "gwt-spec";
/// Substring that uniquely identifies a SPEC body header. We do not run the
/// full regex here because callers only need to decide whether to fetch
/// comments — a positive substring match is enough to trigger the comment
/// fetch path. The actual structural parse still happens later in
/// [`gwt_github::body::SpecBody::parse`].
const SPEC_BODY_HEADER_MARKER: &str = "<!-- gwt-spec id=";
const ISSUE_CACHE_REFRESH_META_FILE: &str = "refresh-meta.json";
pub const ISSUE_CACHE_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IssueCacheRefreshMeta {
    last_full_refresh: String,
    ttl_minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCacheSourceFingerprint {
    pub fingerprint: String,
    pub document_count: usize,
    pub cache_refresh_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCacheSyncOutcome {
    pub refreshed: bool,
    pub source_changed: bool,
    pub before: Option<IssueCacheSourceFingerprint>,
    pub after: Option<IssueCacheSourceFingerprint>,
}

#[derive(Debug)]
struct IssueCacheSourceDocument {
    number: u64,
    title: String,
    body: String,
    state: String,
    labels: Vec<String>,
}

pub fn issue_cache_base_root() -> PathBuf {
    gwt_cache_dir().join("issues")
}

pub fn detached_issue_cache_root() -> PathBuf {
    issue_cache_base_root().join(DETACHED_REPO_CACHE_DIR)
}

pub fn issue_cache_root_for_repo_hash(repo_hash: &RepoHash) -> PathBuf {
    issue_cache_base_root().join(repo_hash.as_str())
}

pub fn issue_cache_root_for_repo_slug(owner: &str, repo: &str) -> PathBuf {
    let remote = format!("https://github.com/{owner}/{repo}.git");
    issue_cache_root_for_repo_hash(&compute_repo_hash(&remote))
}

pub fn issue_cache_root_for_repo_path(repo_path: &Path) -> Option<PathBuf> {
    crate::index_worker::detect_repo_hash(repo_path)
        .map(|repo_hash| issue_cache_root_for_repo_hash(&repo_hash))
}

pub fn issue_cache_root_for_repo_path_or_detached(repo_path: &Path) -> PathBuf {
    issue_cache_root_for_repo_path(repo_path).unwrap_or_else(detached_issue_cache_root)
}

pub fn issue_cache_source_fingerprint(
    cache_root: &Path,
) -> Result<Option<IssueCacheSourceFingerprint>, String> {
    if !cache_root.is_dir() {
        return Ok(None);
    }

    let mut docs = Vec::new();
    let entries = fs::read_dir(cache_root).map_err(|err| err.to_string())?;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        let Ok(number) = name.parse::<u64>() else {
            continue;
        };
        let issue_dir = entry.path();
        let meta_path = issue_dir.join("meta.json");
        if !meta_path.is_file() {
            continue;
        }
        let meta_bytes = match fs::read(&meta_path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let meta: Value = match serde_json::from_slice(&meta_bytes) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let body = fs::read_to_string(issue_dir.join("body.md")).unwrap_or_default();
        let mut labels = match meta.get("labels") {
            Some(Value::String(label)) => vec![label.clone()],
            Some(Value::Array(values)) => values
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        labels.sort();
        docs.push(IssueCacheSourceDocument {
            number,
            title: meta
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            body: body.chars().take(2000).collect(),
            state: meta
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            labels,
        });
    }

    docs.sort_by_key(|doc| doc.number);
    let document_count = docs.len();
    let canonical = docs
        .into_iter()
        .map(|doc| {
            let mut map = BTreeMap::new();
            map.insert("body", Value::String(doc.body));
            map.insert(
                "labels",
                Value::Array(doc.labels.into_iter().map(Value::String).collect()),
            );
            map.insert("number", Value::Number(doc.number.into()));
            map.insert("state", Value::String(doc.state));
            map.insert("title", Value::String(doc.title));
            map
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&canonical).map_err(|err| err.to_string())?;
    let digest = Sha256::digest(&bytes);
    let fingerprint = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Some(IssueCacheSourceFingerprint {
        fingerprint,
        document_count,
        cache_refresh_at: read_issue_cache_refresh_meta(cache_root)
            .map(|meta| meta.last_full_refresh),
    }))
}

pub fn issue_cache_source_changed(
    before: &Option<IssueCacheSourceFingerprint>,
    after: &Option<IssueCacheSourceFingerprint>,
) -> bool {
    before.as_ref().map(|value| &value.fingerprint)
        != after.as_ref().map(|value| &value.fingerprint)
}

pub fn sync_issue_cache_from_remote_if_missing(
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

pub fn sync_issue_cache_from_remote_if_stale(
    repo_path: &Path,
    cache_root: &Path,
    ttl: Duration,
) -> Result<bool, String> {
    if issue_cache_root_for_repo_path(repo_path).is_none() {
        return Ok(false);
    }
    if !issue_cache_refresh_is_stale(cache_root, ttl) {
        return Ok(false);
    }
    sync_issue_cache_from_remote(repo_path, cache_root)?;
    Ok(true)
}

pub fn sync_issue_cache_from_remote_if_stale_with_fingerprint(
    repo_path: &Path,
    cache_root: &Path,
    ttl: Duration,
) -> Result<IssueCacheSyncOutcome, String> {
    let before = issue_cache_source_fingerprint(cache_root)?;
    let refreshed = sync_issue_cache_from_remote_if_stale(repo_path, cache_root, ttl)?;
    let after = issue_cache_source_fingerprint(cache_root)?;
    Ok(IssueCacheSyncOutcome {
        refreshed,
        source_changed: refreshed && issue_cache_source_changed(&before, &after),
        before,
        after,
    })
}

pub fn sync_issue_cache_from_remote_with_fingerprint(
    repo_path: &Path,
    cache_root: &Path,
) -> Result<IssueCacheSyncOutcome, String> {
    let before = issue_cache_source_fingerprint(cache_root)?;
    sync_issue_cache_from_remote(repo_path, cache_root)?;
    let after = issue_cache_source_fingerprint(cache_root)?;
    Ok(IssueCacheSyncOutcome {
        refreshed: true,
        source_changed: issue_cache_source_changed(&before, &after),
        before,
        after,
    })
}

pub fn sync_issue_cache_from_remote(repo_path: &Path, cache_root: &Path) -> Result<(), String> {
    let snapshots = fetch_issue_list_snapshots(repo_path)?;
    if snapshots.is_empty() {
        fs::create_dir_all(cache_root).map_err(|err| err.to_string())?;
        write_issue_cache_refresh_meta(cache_root, ISSUE_CACHE_TTL)?;
        return Ok(());
    }

    let cache = Cache::new(cache_root.to_path_buf());
    for listed_snapshot in &snapshots {
        let snapshot = if is_spec_issue(listed_snapshot) {
            fetch_issue_snapshot(repo_path, listed_snapshot.number)?
        } else {
            listed_snapshot.clone()
        };
        cache
            .write_snapshot(&snapshot)
            .map_err(|err| format!("write issue cache: {err}"))?;
    }
    write_issue_cache_refresh_meta(cache_root, ISSUE_CACHE_TTL)?;
    Ok(())
}

fn gh_repo_cwd(repo_path: &Path) -> PathBuf {
    crate::index_worker::resolve_project_index_repo_root(repo_path)
        .unwrap_or_else(|| repo_path.to_path_buf())
}

pub fn issue_cache_has_entries(cache_root: &Path) -> bool {
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

/// Decide whether an Issue should be fetched as a SPEC (with comments) during
/// `sync_issue_cache_from_remote`. The label `gwt-spec` is the primary signal,
/// but the body header is also authoritative — without this fallback, an
/// Issue whose `gwt-spec` label has been removed (or was never applied) but
/// whose body still carries section markers referencing comments would be
/// cached without those comments, and the next [`gwt_github::cache::Cache::write_snapshot`]
/// would fail with `MissingComment`.
fn is_spec_issue(snapshot: &IssueSnapshot) -> bool {
    snapshot.labels.iter().any(|label| label == SPEC_LABEL)
        || snapshot.body.contains(SPEC_BODY_HEADER_MARKER)
}

fn gh_executable() -> std::ffi::OsString {
    if let Some(path) = std::env::var_os("GWT_TEST_GH") {
        return path;
    }
    std::ffi::OsString::from("gh")
}

fn issue_cache_refresh_meta_path(cache_root: &Path) -> PathBuf {
    cache_root.join(ISSUE_CACHE_REFRESH_META_FILE)
}

fn read_issue_cache_refresh_meta(cache_root: &Path) -> Option<IssueCacheRefreshMeta> {
    let path = issue_cache_refresh_meta_path(cache_root);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_issue_cache_refresh_meta(cache_root: &Path, ttl: Duration) -> Result<(), String> {
    fs::create_dir_all(cache_root).map_err(|err| err.to_string())?;
    let ttl_minutes = std::cmp::max(1, ttl.as_secs() / 60);
    let payload = IssueCacheRefreshMeta {
        last_full_refresh: Utc::now().to_rfc3339(),
        ttl_minutes,
    };
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|err| err.to_string())?;
    write_atomic(&issue_cache_refresh_meta_path(cache_root), &bytes).map_err(|err| err.to_string())
}

fn issue_cache_refresh_is_stale(cache_root: &Path, ttl: Duration) -> bool {
    if !issue_cache_has_entries(cache_root) {
        return true;
    }
    let Some(meta) = read_issue_cache_refresh_meta(cache_root) else {
        return true;
    };
    let Ok(last) = DateTime::parse_from_rfc3339(&meta.last_full_refresh) else {
        return true;
    };
    Utc::now()
        .signed_duration_since(last.with_timezone(&Utc))
        .to_std()
        .map_or(true, |age| age >= ttl)
}

/// SPEC-2017 US-8 — Apply label add / remove operations to a GitHub
/// Issue via `gh issue edit`. The flags are passed in a single command
/// so the API call is atomic; either all labels are written or none
/// are. `labels_to_add` and `labels_to_remove` are deduplicated by the
/// caller (`update_knowledge_phase`).
pub(crate) fn write_issue_labels_via_gh(
    repo_path: &Path,
    issue_number: u64,
    labels_to_add: &[String],
    labels_to_remove: &[String],
) -> Result<(), String> {
    if labels_to_add.is_empty() && labels_to_remove.is_empty() {
        return Ok(());
    }
    let mut command = gwt_core::process::hidden_command(gh_executable());
    command.args(["issue", "edit", &issue_number.to_string()]);
    for label in labels_to_add {
        command.arg("--add-label").arg(label);
    }
    for label in labels_to_remove {
        command.arg("--remove-label").arg(label);
    }
    let output = command
        .current_dir(gh_repo_cwd(repo_path))
        .output()
        .map_err(|err| format!("gh issue edit #{issue_number}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh issue edit #{issue_number}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn fetch_issue_list_snapshots(repo_path: &Path) -> Result<Vec<IssueSnapshot>, String> {
    let output = gwt_core::process::hidden_command(gh_executable())
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
        .current_dir(gh_repo_cwd(repo_path))
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

fn fetch_issue_snapshot(repo_path: &Path, number: IssueNumber) -> Result<IssueSnapshot, String> {
    let output = gwt_core::process::hidden_command(gh_executable())
        .args([
            "issue",
            "view",
            &number.0.to_string(),
            "--json",
            "number,title,body,labels,state,updatedAt,comments",
        ])
        .current_dir(gh_repo_cwd(repo_path))
        .output()
        .map_err(|err| format!("gh issue view #{number}: {err}", number = number.0))?;
    if !output.status.success() {
        return Err(format!(
            "gh issue view #{number}: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            number = number.0
        ));
    }

    parse_issue_snapshot(&String::from_utf8_lossy(&output.stdout), number)
}

fn parse_issue_state(value: Option<&str>) -> IssueState {
    match value {
        Some("CLOSED" | "closed") => IssueState::Closed,
        _ => IssueState::Open,
    }
}

fn parse_issue_labels(issue: &Value) -> Vec<String> {
    issue
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
        .unwrap_or_default()
}

fn parse_comment_id(comment: &Value) -> Option<u64> {
    if let Some(id) = comment
        .get("databaseId")
        .and_then(serde_json::Value::as_u64)
    {
        return Some(id);
    }
    if let Some(id) = comment.get("id").and_then(serde_json::Value::as_u64) {
        return Some(id);
    }
    let url = comment.get("url").and_then(|value| value.as_str())?;
    url.rsplit_once("issuecomment-")
        .and_then(|(_, raw)| raw.parse::<u64>().ok())
}

fn parse_issue_snapshot(json: &str, number: IssueNumber) -> Result<IssueSnapshot, String> {
    let issue: Value = serde_json::from_str(json).map_err(|err| err.to_string())?;
    let actual_number = issue
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("issue #{number} missing number field", number = number.0))?;
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
    let labels = parse_issue_labels(&issue);
    let state = parse_issue_state(issue.get("state").and_then(|value| value.as_str()));
    let issue_updated_at = issue
        .get("updatedAt")
        .and_then(|value| value.as_str())
        .unwrap_or("1970-01-01T00:00:00Z")
        .to_string();
    let comments = issue
        .get("comments")
        .and_then(|value| value.as_array())
        .map(|comments| {
            comments
                .iter()
                .filter_map(|comment| {
                    let id = parse_comment_id(comment)?;
                    let body = comment
                        .get("body")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let updated_at = comment
                        .get("updatedAt")
                        .or_else(|| comment.get("createdAt"))
                        .and_then(|value| value.as_str())
                        .unwrap_or(issue_updated_at.as_str())
                        .to_string();
                    Some(CommentSnapshot {
                        id: CommentId(id),
                        body,
                        updated_at: UpdatedAt::new(updated_at),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(IssueSnapshot {
        number: IssueNumber(actual_number),
        title,
        body,
        labels,
        state,
        updated_at: UpdatedAt::new(issue_updated_at),
        comments,
    })
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
            let labels = parse_issue_labels(&issue);
            let state = parse_issue_state(issue.get("state").and_then(|value| value.as_str()));
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
    #[cfg(any(target_os = "windows", unix))]
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
echo [{\"number\":7,\"title\":\"Cached issue\",\"body\":\"Body\",\"labels\":[{\"name\":\"bug\"}],\"state\":\"OPEN\",\"url\":\"https://example.test/issues/7\",\"updatedAt\":\"2026-04-20T00:00:00Z\"}]\r\n\
exit /b 0\r\n",
        )
        .expect("write fake gh");
        gwt_core::process::hidden_command("git")
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
        assert_eq!(entry.snapshot.labels, vec!["bug".to_string()]);

        env::set_var("FAKE_GH_MODE", "empty");
        sync_issue_cache_from_remote(&repo_path, &empty_cache_root).expect("empty sync succeeds");
        assert!(empty_cache_root.is_dir());
        let entries = fs::read_dir(&empty_cache_root)
            .expect("read empty cache")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect empty cache entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].file_name().to_string_lossy(),
            ISSUE_CACHE_REFRESH_META_FILE
        );

        env::set_var("FAKE_GH_MODE", "fail");
        let err = sync_issue_cache_from_remote(&repo_path, &cache_root).unwrap_err();
        assert!(err.contains("gh issue list: gh api down"));

        match old_gh {
            Some(value) => env::set_var("GWT_TEST_GH", value),
            None => env::remove_var("GWT_TEST_GH"),
        }
        env::remove_var("FAKE_GH_MODE");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn sync_issue_cache_from_remote_fetches_full_spec_snapshots_with_comment_sections() {
        let _guard = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempdir().expect("tempdir");
        let repo_path = temp.path().join("repo");
        let cache_root = temp.path().join("cache");
        fs::create_dir_all(&repo_path).expect("create repo path");
        let fake_gh = repo_path.join("gh.cmd");
        fs::write(
            &fake_gh,
            "@echo off\r\n\
if /I \"%1 %2\"==\"issue list\" (\r\n\
  echo [{\"number\":7,\"title\":\"Cached spec\",\"body\":\"<!-- gwt-spec id=7 version=1 -->\\n<!-- sections:\\nplan=comment:700\\nspec=body\\ntasks=body\\n-->\\n\\n<!-- artifact:spec BEGIN -->\\nSpec body\\n<!-- artifact:spec END -->\\n\\n<!-- artifact:tasks BEGIN -->\\n- [ ] T-001\\n<!-- artifact:tasks END -->\",\"labels\":[{\"name\":\"gwt-spec\"}],\"state\":\"OPEN\",\"url\":\"https://example.test/issues/7\",\"updatedAt\":\"2026-04-20T00:00:00Z\"}]\r\n\
  exit /b 0\r\n\
)\r\n\
if /I \"%1 %2\"==\"issue view\" (\r\n\
  echo {\"number\":7,\"title\":\"Cached spec\",\"body\":\"<!-- gwt-spec id=7 version=1 -->\\n<!-- sections:\\nplan=comment:700\\nspec=body\\ntasks=body\\n-->\\n\\n<!-- artifact:spec BEGIN -->\\nSpec body\\n<!-- artifact:spec END -->\\n\\n<!-- artifact:tasks BEGIN -->\\n- [ ] T-001\\n<!-- artifact:tasks END -->\",\"labels\":[{\"name\":\"gwt-spec\"}],\"state\":\"OPEN\",\"updatedAt\":\"2026-04-20T00:00:00Z\",\"comments\":[{\"id\":\"IC_kwDOExample\",\"url\":\"https://github.com/example/repo/issues/7#issuecomment-700\",\"body\":\"<!-- artifact:plan BEGIN -->\\nPlan body\\n<!-- artifact:plan END -->\",\"createdAt\":\"2026-04-20T00:00:00Z\"}]}\r\n\
  exit /b 0\r\n\
)\r\n\
>&2 echo unexpected gh invocation %*\r\n\
exit /b 1\r\n",
        )
        .expect("write fake gh");
        gwt_core::process::hidden_command("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .status()
            .expect("git init");

        let old_gh = env::var_os("GWT_TEST_GH");
        env::set_var("GWT_TEST_GH", &fake_gh);

        sync_issue_cache_from_remote(&repo_path, &cache_root).expect("sync success");
        let cache = Cache::new(cache_root);
        let entry = cache
            .load_entry(IssueNumber(7))
            .expect("cached spec entry should exist");
        assert_eq!(
            entry
                .spec_body
                .sections
                .get(&gwt_github::SectionName("plan".to_string())),
            Some(&"Plan body".to_string())
        );
        assert_eq!(entry.snapshot.comments.len(), 1);
        assert_eq!(entry.snapshot.comments[0].id, gwt_github::CommentId(700));

        match old_gh {
            Some(value) => env::set_var("GWT_TEST_GH", value),
            None => env::remove_var("GWT_TEST_GH"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn sync_remote_detects_spec_via_body_header_when_label_missing() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempdir().expect("tempdir");
        let repo_path = temp.path().join("repo");
        let cache_root = temp.path().join("cache");
        fs::create_dir_all(&repo_path).expect("create repo path");

        let body_escaped = "<!-- gwt-spec id=42 version=1 -->\\n\
            <!-- sections:\\nplan=comment:777\\nspec=body\\ntasks=body\\n-->\\n\\n\
            <!-- artifact:spec BEGIN -->\\nSpec body\\n<!-- artifact:spec END -->\\n\\n\
            <!-- artifact:tasks BEGIN -->\\n- [ ] T-001\\n<!-- artifact:tasks END -->";
        let list_json = format!(
            r#"[{{"number":42,"title":"Stripped label spec","body":"{body_escaped}","labels":[],"state":"OPEN","url":"https://example.test/issues/42","updatedAt":"2026-05-20T00:00:00Z"}}]"#,
        );
        let view_json = format!(
            r#"{{"number":42,"title":"Stripped label spec","body":"{body_escaped}","labels":[],"state":"OPEN","updatedAt":"2026-05-20T00:00:00Z","comments":[{{"id":"IC_kwDOTest","url":"https://github.com/example/repo/issues/42#issuecomment-777","body":"<!-- artifact:plan BEGIN -->\nPlan body for stripped\n<!-- artifact:plan END -->","createdAt":"2026-05-20T00:00:00Z"}}]}}"#,
        );
        let fake_gh = repo_path.join("fake-gh");
        let script = format!(
            "#!/bin/sh\n\
if [ \"$1 $2\" = \"issue list\" ]; then\n\
  cat <<'JSON'\n\
{list_json}\n\
JSON\n\
  exit 0\n\
fi\n\
if [ \"$1 $2\" = \"issue view\" ]; then\n\
  cat <<'JSON'\n\
{view_json}\n\
JSON\n\
  exit 0\n\
fi\n\
echo \"unexpected gh invocation $*\" >&2\n\
exit 1\n",
        );
        fs::write(&fake_gh, script).expect("write fake gh");
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");

        gwt_core::process::hidden_command("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .status()
            .expect("git init");

        let old_gh = env::var_os("GWT_TEST_GH");
        env::set_var("GWT_TEST_GH", &fake_gh);

        let result = sync_issue_cache_from_remote(&repo_path, &cache_root);

        match old_gh {
            Some(value) => env::set_var("GWT_TEST_GH", value),
            None => env::remove_var("GWT_TEST_GH"),
        }

        result.expect("sync should succeed when body header marks the issue as a spec");

        let cache = Cache::new(cache_root);
        let entry = cache
            .load_entry(IssueNumber(42))
            .expect("cached entry must exist");
        assert_eq!(
            entry
                .spec_body
                .sections
                .get(&gwt_github::SectionName("plan".to_string())),
            Some(&"Plan body for stripped".to_string()),
            "plan section content from comment must be present even without gwt-spec label"
        );
        assert_eq!(entry.snapshot.comments.len(), 1);
        assert_eq!(entry.snapshot.comments[0].id, gwt_github::CommentId(777));
    }

    #[cfg(unix)]
    #[test]
    fn sync_remote_uses_child_bare_repo_cwd_for_workspace_home() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempdir().expect("tempdir");
        let workspace_home = temp.path().join("workspace");
        let bare_repo = workspace_home.join("repo.git");
        let cache_root = temp.path().join("cache");
        fs::create_dir_all(&workspace_home).expect("create workspace home");
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "--bare", bare_repo.to_str().unwrap()])
            .output()
            .expect("git init bare");
        assert!(
            init.status.success(),
            "git init bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        let remote = gwt_core::process::hidden_command("git")
            .args([
                "-C",
                bare_repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                "https://github.com/example/workspace-home.git",
            ])
            .output()
            .expect("git remote add");
        assert!(
            remote.status.success(),
            "git remote add failed: {}",
            String::from_utf8_lossy(&remote.stderr)
        );

        let expected_cwd = dunce::canonicalize(&bare_repo).expect("canonical bare repo");
        let cwd_log = temp.path().join("gh-cwd.log");
        let fake_gh = temp.path().join("fake-gh");
        fs::write(
            &fake_gh,
            format!(
                "#!/bin/sh\n\
printf '%s\\n' \"$PWD\" > '{}'\n\
if [ \"$PWD\" != '{}' ]; then\n\
  printf '%s\\n' \"wrong cwd: $PWD\" >&2\n\
  exit 1\n\
fi\n\
if [ \"$1 $2\" = \"issue list\" ]; then\n\
  printf '%s\\n' '[{{\"number\":43,\"title\":\"Workspace issue\",\"body\":\"Body\",\"labels\":[{{\"name\":\"bug\"}}],\"state\":\"OPEN\",\"url\":\"https://example.test/issues/43\",\"updatedAt\":\"2026-05-23T00:00:00Z\"}}]'\n\
  exit 0\n\
fi\n\
printf '%s\\n' \"unexpected gh invocation $*\" >&2\n\
exit 1\n",
                cwd_log.display(),
                expected_cwd.display()
            ),
        )
        .expect("write fake gh");
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
        let old_gh = env::var_os("GWT_TEST_GH");
        env::set_var("GWT_TEST_GH", &fake_gh);

        let result = sync_issue_cache_from_remote(&workspace_home, &cache_root);

        match old_gh {
            Some(value) => env::set_var("GWT_TEST_GH", value),
            None => env::remove_var("GWT_TEST_GH"),
        }

        result.expect("workspace home sync should use child bare repo cwd");
        assert_eq!(
            fs::read_to_string(&cwd_log).expect("read cwd log").trim(),
            expected_cwd.display().to_string()
        );
        let entry = Cache::new(cache_root)
            .load_entry(IssueNumber(43))
            .expect("cached issue entry should exist");
        assert_eq!(entry.snapshot.title, "Workspace issue");
    }

    #[test]
    fn issue_cache_refresh_is_stale_tracks_cache_metadata() {
        let temp = tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        fs::create_dir_all(cache_root.join("7")).expect("create issue cache entry");

        assert!(
            issue_cache_refresh_is_stale(&cache_root, ISSUE_CACHE_TTL),
            "cache without refresh metadata should be stale",
        );

        write_issue_cache_refresh_meta(&cache_root, Duration::from_secs(60))
            .expect("write refresh meta");
        assert!(
            !issue_cache_refresh_is_stale(&cache_root, Duration::from_secs(60)),
            "fresh refresh metadata should suppress stale refresh",
        );

        let stale_meta = IssueCacheRefreshMeta {
            last_full_refresh: (Utc::now() - chrono::Duration::minutes(30)).to_rfc3339(),
            ttl_minutes: 15,
        };
        let bytes = serde_json::to_vec_pretty(&stale_meta).expect("serialize stale meta");
        write_atomic(&issue_cache_refresh_meta_path(&cache_root), &bytes)
            .expect("write stale refresh meta");
        assert!(
            issue_cache_refresh_is_stale(&cache_root, ISSUE_CACHE_TTL),
            "expired refresh metadata should mark cache stale",
        );
    }

    #[test]
    fn issue_cache_source_fingerprint_tracks_indexed_issue_fields_only() {
        let temp = tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        let snapshot = IssueSnapshot {
            number: IssueNumber(7),
            title: "Cache freshness".to_string(),
            body: "body".to_string(),
            labels: vec!["bug".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-05-23T00:00:00Z"),
            comments: vec![],
        };
        Cache::new(cache_root.clone())
            .write_snapshot(&snapshot)
            .expect("write snapshot");

        let initial = issue_cache_source_fingerprint(&cache_root)
            .expect("initial fingerprint")
            .expect("source snapshot");
        assert_eq!(initial.document_count, 1);
        assert_eq!(
            initial.fingerprint, "0c31a39b484318da3ec39b51fc5e073334bb3d64bba3886fb0ecff50b400d915",
            "Rust source fingerprint must match the Python runner metadata algorithm",
        );

        fs::write(cache_root.join("7").join("linked_prs.json"), "[]").expect("linked prs");
        let linked_prs = issue_cache_source_fingerprint(&cache_root)
            .expect("linked prs fingerprint")
            .expect("source snapshot");
        assert_eq!(
            linked_prs.fingerprint, initial.fingerprint,
            "linked PR cache is not part of the Issue search source",
        );

        let mut changed = snapshot.clone();
        changed.state = IssueState::Closed;
        Cache::new(cache_root.clone())
            .write_snapshot(&changed)
            .expect("write changed snapshot");
        let after_state = issue_cache_source_fingerprint(&cache_root)
            .expect("changed fingerprint")
            .expect("source snapshot");
        assert_ne!(
            after_state.fingerprint, initial.fingerprint,
            "indexed Issue state changes must invalidate the Issue search index",
        );
    }
}
