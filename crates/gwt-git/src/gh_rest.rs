//! Paged REST reads through `gh api` (SPEC #4093 FR-002).
//!
//! `gh pr list` / `gh issue list` run on GraphQL, whose hourly budget is
//! charged in points that grow with the page size, so one `--limit 999` list
//! can burn hundreds of points. The REST `core` budget is charged per request
//! regardless of size (one page of 100 rows = one request), and it is a
//! separate 5,000/h pool. Ordinary lists retain a bounded REST request cost;
//! the queue label collection opts into complete pagination (#5080 AC-1).
//! Both paths spend zero GraphQL points.

use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    hash::{Hash, Hasher},
    path::Path,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bound unchanged Monitor inventories to one live read per four 30s scans.
/// Queue-labelled Issue pages and authoritative per-resource reads bypass it.
const LIST_CACHE_TTL_SECS: i64 = 120;

#[derive(Serialize, Deserialize)]
struct CachedListResponse {
    key: String,
    fetched_at: DateTime<Utc>,
    stdout: String,
}

pub(crate) fn cached_list_response_with(
    cache_dir: &Path,
    key: &str,
    now: DateTime<Utc>,
    fetch: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let path = cache_dir.join(format!("{:016x}.json", hasher.finish()));
    if let Some(cached) = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CachedListResponse>(&bytes).ok())
    {
        let age = now.signed_duration_since(cached.fetched_at).num_seconds();
        if cached.key == key
            && (0..LIST_CACHE_TTL_SECS).contains(&age)
            && valid_list_response(&cached.stdout)
        {
            return Ok(cached.stdout);
        }
    }
    // Expired or unreadable entries never hide a failed live read. Cache only
    // valid array payloads, with or without the `gh api --include` headers.
    let stdout = fetch()?;
    if valid_list_response(&stdout) {
        let cached = CachedListResponse {
            key: key.to_string(),
            fetched_at: now,
            stdout: stdout.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&cached) {
            let _ = std::fs::create_dir_all(cache_dir);
            let _ = gwt_core::atomic_file::write_atomic(&path, &bytes);
        }
    }
    Ok(stdout)
}

fn valid_list_response(stdout: &str) -> bool {
    let body = match stdout
        .split_once("\n\r\n")
        .or_else(|| stdout.split_once("\n\n"))
    {
        Some((headers, body)) => {
            let status = headers.lines().next().unwrap_or_default();
            if !status.starts_with("HTTP/")
                || status
                    .split_whitespace()
                    .nth(1)
                    .and_then(|code| code.parse::<u16>().ok())
                    .is_none_or(|code| !(200..300).contains(&code))
            {
                return false;
            }
            body
        }
        None => stdout,
    };
    serde_json::from_str::<Value>(body).is_ok_and(|value| value.is_array())
}

/// Rows per REST page (GitHub's maximum).
pub const REST_PAGE_SIZE: usize = 100;

/// Upper bound on REST requests one paged read may spend. Together with
/// [`REST_PAGE_SIZE`] this keeps the 1,000-row ceiling the GraphQL lists had,
/// so callers that treat a full list as "possibly incomplete" keep working.
pub const REST_MAX_PAGES_PER_READ: usize = 10;

/// What one paged read returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestPages {
    pub rows: Vec<Value>,
    /// REST requests spent.
    pub requests: usize,
    /// `true` when the page budget ran out with a continuation still unread.
    pub capped: bool,
}

/// The `gh api` path for `page` of `endpoint` (a path with or without a query).
pub fn page_endpoint(endpoint: &str, page: usize) -> String {
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!("{endpoint}{separator}per_page={REST_PAGE_SIZE}&page={page}")
}

/// Read `endpoint` until there is no HTTP Link continuation or the page budget.
/// `fetch` runs one `gh api <path> --include` and answers its stdout, or the failure
/// text; a failed page fails the whole read so a partial list never looks
/// complete.
pub fn read_pages_with<F>(endpoint: &str, fetch: F) -> Result<RestPages, String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    read_pages_with_budget(endpoint, Some(REST_MAX_PAGES_PER_READ), fetch)
}

/// Read a label-scoped collection to its final Link page. This opt-in reader
/// does not change the request cap of ordinary lists. A failed page or repeated
/// continuation fails the whole read rather than returning a partial collection.
pub fn read_complete_pages_with<F>(endpoint: &str, fetch: F) -> Result<RestPages, String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    read_pages_with_budget(endpoint, None, fetch)
}

fn read_pages_with_budget<F>(
    endpoint: &str,
    max_pages: Option<usize>,
    mut fetch: F,
) -> Result<RestPages, String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    let mut rows = Vec::new();
    let mut requests = 0;
    let mut next = Some(page_endpoint(endpoint, 1));
    let mut seen = HashSet::new();
    while let Some(path) = next.take() {
        if max_pages.is_none() && !seen.insert(path.clone()) {
            return Err(format!("gh api {path}: repeated HTTP Link continuation"));
        }
        let stdout = fetch(&path)?;
        requests += 1;
        let (headers, body) = stdout
            .split_once("\n\r\n")
            .or_else(|| stdout.split_once("\n\n"))
            .ok_or_else(|| format!("gh api {path}: missing HTTP headers"))?;
        let mut lines = headers.lines();
        let status = lines.next().unwrap_or_default();
        if !status.starts_with("HTTP/")
            || status
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse::<u16>().ok())
                .is_none_or(|code| !(200..300).contains(&code))
        {
            return Err(format!("gh api {path}: invalid HTTP status {status}"));
        }
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("link") {
                    next = next_link(value)?.or(next);
                }
            }
        }
        let page_rows: Vec<Value> =
            serde_json::from_str(body).map_err(|e| format!("gh api {path} JSON: {e}"))?;
        rows.extend(page_rows);
        if max_pages == Some(requests) {
            break;
        }
    }
    Ok(RestPages {
        rows,
        requests,
        capped: next.is_some(),
    })
}

fn next_link(value: &str) -> Result<Option<String>, String> {
    for link in value.split(',') {
        let (target, parameters) = link
            .trim()
            .split_once(';')
            .ok_or_else(|| "gh api: invalid HTTP Link header".to_string())?;
        let target = target
            .trim()
            .strip_prefix('<')
            .and_then(|target| target.strip_suffix('>'))
            .filter(|target| !target.is_empty())
            .ok_or_else(|| "gh api: invalid HTTP Link target".to_string())?;
        let mut has_relation = false;
        for parameter in parameters.split(';') {
            let (name, value) = parameter
                .trim()
                .split_once('=')
                .unwrap_or((parameter.trim(), ""));
            if !name.trim().eq_ignore_ascii_case("rel") {
                continue;
            }
            has_relation = true;
            let value = value.trim();
            let relation = if let Some(quoted) = value.strip_prefix('"') {
                quoted.strip_suffix('"').unwrap_or_default()
            } else {
                value
            };
            if relation.is_empty() {
                return Err("gh api: invalid HTTP Link relation".to_string());
            }
            if relation.split_ascii_whitespace().any(|rel| rel == "next") {
                return Ok(Some(target.to_string()));
            }
        }
        if !has_relation {
            return Err("gh api: missing HTTP Link relation".to_string());
        }
    }
    Ok(None)
}

/// One Issue as `GET /repos/{owner}/{repo}/issues` returns it, reduced to the
/// fields gwt reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestIssueRow {
    pub number: u64,
    pub title: String,
    /// `OPEN` / `CLOSED`, normalized to the casing the GraphQL list used.
    pub state: String,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub body: Option<String>,
    pub url: String,
    pub updated_at: Option<String>,
}

/// Reduce REST issue rows. Pull requests (the endpoint returns them too) and
/// rows without a number are dropped. Field names accept the GraphQL spelling
/// as a fallback (`updatedAt`, `url`) so the two list shapes reduce the same
/// way.
pub fn parse_issue_rows(rows: &[Value]) -> Vec<RestIssueRow> {
    rows.iter()
        .filter(|row| row.get("pull_request").is_none())
        .filter_map(|row| {
            let number = row.get("number")?.as_u64()?;
            Some(RestIssueRow {
                number,
                title: text(row, &["title"]).unwrap_or_default(),
                state: text(row, &["state"])
                    .map(|state| state.to_ascii_uppercase())
                    .unwrap_or_else(|| "OPEN".to_string()),
                labels: row
                    .get("labels")
                    .and_then(Value::as_array)
                    .map(|labels| {
                        labels
                            .iter()
                            .filter_map(|label| text(label, &["name"]))
                            .collect()
                    })
                    .unwrap_or_default(),
                assignee: row
                    .get("assignees")
                    .and_then(Value::as_array)
                    .and_then(|assignees| assignees.first())
                    .and_then(|assignee| text(assignee, &["login"])),
                body: row.get("body").and_then(Value::as_str).map(String::from),
                url: text(row, &["html_url", "url"]).unwrap_or_default(),
                updated_at: text(row, &["updated_at", "updatedAt"]),
            })
        })
        .collect()
}

/// The first non-empty string among `keys` of `row`.
fn text(row: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        row.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(String::from)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_5603_unchanged_lists_halve_hourly_rest_consumption() {
        let start = chrono::DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        for implementing in [false, true] {
            let cache = tempfile::tempdir().unwrap();
            let mut calls = 0;
            for tick in 0..120 {
                let now = start + chrono::Duration::seconds(tick * 30);
                for key in ["general-issues", "closed-pulls"]
                    .into_iter()
                    .chain(implementing.then_some("open-pulls"))
                {
                    let response = cached_list_response_with(cache.path(), key, now, || {
                        calls += 1;
                        Ok("[]".to_string())
                    })
                    .unwrap();
                    assert_eq!(response, "[]");
                }
                // Queue membership and payloads remain live on every scan.
                calls += 1;
            }
            let baseline = if implementing { 480 } else { 360 };
            assert_eq!(calls, if implementing { 210 } else { 180 });
            assert!(calls <= baseline / 2);
            println!("unchanged Monitor lists: implementing={implementing}, {baseline} -> {calls} requests/hour");

            // HTTP Link continuations are distinct cached pages too.
            let mut continuation_calls = 0;
            for tick in 0..120 {
                cached_list_response_with(
                    cache.path(),
                    "https://api.github.com/repos/o/r/issues?per_page=100&page=2",
                    start + chrono::Duration::seconds(tick * 30),
                    || {
                        continuation_calls += 1;
                        Ok("HTTP/2.0 200 OK\n\r\n[]".into())
                    },
                )
                .unwrap();
            }
            assert_eq!(continuation_calls, 30);
            assert!(calls + continuation_calls <= (baseline + 120) / 2);
        }
    }

    #[test]
    fn issue_5603_expired_list_cache_does_not_hide_a_failed_refresh() {
        let cache = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now();
        cached_list_response_with(cache.path(), "issues", now, || Ok("[]".into())).unwrap();
        let fresh = cached_list_response_with(cache.path(), "issues", now, || {
            Err("cache should answer".to_string())
        });
        assert_eq!(fresh.unwrap(), "[]");
        let expired = cached_list_response_with(
            cache.path(),
            "issues",
            now + chrono::Duration::seconds(LIST_CACHE_TTL_SECS),
            || Err("live read failed".to_string()),
        );
        assert_eq!(expired.unwrap_err(), "live read failed");
    }

    #[test]
    fn issue_5603_invalid_http_response_never_enters_list_cache() {
        let cache = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now();
        let mut calls = 0;
        for _ in 0..2 {
            cached_list_response_with(cache.path(), "issues", now, || {
                calls += 1;
                Ok("HTTP/2.0 503 Service Unavailable\n\r\n[]".into())
            })
            .unwrap();
        }
        assert_eq!(
            calls, 2,
            "an invalid HTTP status cannot become a cached observation"
        );
    }

    fn rows(count: usize, offset: usize) -> String {
        let rows = (0..count)
            .map(|i| serde_json::json!({"number": offset + i, "title": format!("#{}", offset + i)}))
            .collect::<Vec<_>>();
        serde_json::to_string(&rows).unwrap()
    }

    #[test]
    fn page_endpoint_appends_pagination_to_paths_with_and_without_a_query() {
        assert_eq!(
            page_endpoint("repos/{owner}/{repo}/issues?state=open", 3),
            "repos/{owner}/{repo}/issues?state=open&per_page=100&page=3"
        );
        assert_eq!(
            page_endpoint("repos/o/r/pulls", 1),
            "repos/o/r/pulls?per_page=100&page=1"
        );
    }

    #[test]
    fn read_pages_follows_next_through_short_and_empty_pages() {
        let mut calls = Vec::new();
        let next = "https://api.github.com/repos/o/r/issues?state=open&per_page=30&page=2";
        let last = "https://api.github.com/repos/o/r/issues?state=open&per_page=30&page=3";
        let pages = read_pages_with("repos/o/r/issues?state=open", |path| {
            calls.push(path.to_string());
            Ok(match calls.len() {
                1 => format!("HTTP/2.0 200 OK\nLink: <{next}>; rel=\"next\", <{last}>; rel=\"last\"\r\n\r\n{}", rows(1, 0)),
                2 => format!("HTTP/2.0 200 OK\nlInK: <{last}>; rel=\"next\"\r\n\r\n[]"),
                // A full final page without next is still complete.
                _ => format!("HTTP/2.0 200 OK\n\r\n{}", rows(REST_PAGE_SIZE, 1)),
            })
        })
        .unwrap();
        assert_eq!(pages.requests, 3);
        assert_eq!(pages.rows.len(), 101);
        assert!(!pages.capped);
        assert_eq!(&calls[1..], &[next, last]);
    }

    #[test]
    fn read_pages_is_bounded_by_the_page_budget_and_reports_the_cap() {
        // SPEC #4093 AC-3: a list of any size costs at most
        // REST_MAX_PAGES_PER_READ requests, all on the REST budget.
        let mut calls = 0;
        let pages = read_pages_with("repos/o/r/issues?state=all", |_| {
            calls += 1;
            Ok(format!("HTTP/2.0 200 OK\nLink: <https://api.github.com/repos/o/r/issues?page={}>; rel=\"next\"\r\n\r\n{}", calls + 1, rows(REST_PAGE_SIZE, (calls - 1) * 100)))
        })
        .unwrap();
        assert_eq!(calls, REST_MAX_PAGES_PER_READ);
        assert_eq!(pages.requests, REST_MAX_PAGES_PER_READ);
        assert_eq!(pages.rows.len(), REST_MAX_PAGES_PER_READ * REST_PAGE_SIZE);
        assert!(pages.capped, "next remains after the request budget");
    }

    #[test]
    fn queue_label_complete_pages_read_past_the_regular_page_budget() {
        let mut calls = 0;
        let pages = read_complete_pages_with("repos/o/r/issues?labels=gwt-queued", |_| {
            calls += 1;
            let link = if calls <= REST_MAX_PAGES_PER_READ {
                format!(
                    "Link: <https://api.github.com/repos/o/r/issues?page={}>; rel=\"next\"\r\n",
                    calls + 1
                )
            } else {
                String::new()
            };
            Ok(format!("HTTP/2.0 200 OK\n{link}\r\n{}", rows(1, calls)))
        })
        .unwrap();
        assert_eq!(calls, REST_MAX_PAGES_PER_READ + 1);
        assert_eq!(pages.rows.len(), REST_MAX_PAGES_PER_READ + 1);
        assert!(!pages.capped);
    }

    #[test]
    fn queue_label_complete_pages_reject_a_repeated_continuation() {
        let mut calls = 0;
        let error = read_complete_pages_with("repos/o/r/issues?labels=gwt-queued", |_| {
            calls += 1;
            Ok("HTTP/2.0 200 OK\nLink: <https://api.github.com/repos/o/r/issues?page=2>; rel=\"next\"\r\n\r\n[]".to_string())
        })
        .unwrap_err();
        assert!(error.contains("repeated"), "{error}");
        assert_eq!(calls, 2);
    }

    #[test]
    fn queue_label_complete_pages_fail_on_an_error_after_the_regular_budget() {
        let mut calls = 0;
        let error = read_complete_pages_with("repos/o/r/issues?labels=gwt-queued", |_| {
            calls += 1;
            if calls > REST_MAX_PAGES_PER_READ {
                return Err("HTTP 502 on the queued continuation".to_string());
            }
            Ok(format!("HTTP/2.0 200 OK\nLink: <https://api.github.com/repos/o/r/issues?page={}>; rel=\"next\"\r\n\r\n[]", calls + 1))
        })
        .unwrap_err();
        assert!(error.contains("HTTP 502"), "{error}");
        assert_eq!(calls, REST_MAX_PAGES_PER_READ + 1);
    }

    #[test]
    fn a_failed_or_unparseable_page_fails_the_whole_read() {
        let failure =
            read_pages_with("repos/o/r/issues", |_| Err("HTTP 502".to_string())).unwrap_err();
        assert!(failure.contains("HTTP 502"), "{failure}");

        let mut calls = 0;
        let failure = read_pages_with("repos/o/r/issues", |_| {
            calls += 1;
            Ok(if calls == 1 {
                format!("HTTP/2.0 200 OK\nLink: <https://api.github.com/repos/o/r/issues?page=2>; rel=\"next\"\r\n\r\n{}", rows(1, 0))
            } else {
                "HTTP/2.0 200 OK\n\r\nnot json".to_string()
            })
        })
        .unwrap_err();
        assert!(failure.contains("JSON"), "{failure}");
        assert_eq!(calls, 2, "a bad continuation fails the whole read");

        let failure = read_pages_with("repos/o/r/issues", |_| Ok("[]".to_string())).unwrap_err();
        assert!(failure.contains("HTTP"), "headers are required: {failure}");

        let failure = read_pages_with("repos/o/r/issues", |_| {
            Ok("HTTP/2.0 200 OK\nLink: <https://api.github.com/repos/o/r/issues?page=2>; rel\r\n\r\n[]".to_string())
        }).unwrap_err();
        assert!(
            failure.contains("Link"),
            "invalid continuation must fail: {failure}"
        );
    }

    #[test]
    fn issue_rows_drop_pull_requests_and_accept_both_field_spellings() {
        let rows: Vec<Value> = serde_json::from_str(
            r#"[
              {"number": 1, "title": "Bug", "state": "open", "labels": [{"name": "bug"}],
               "assignees": [{"login": "alice"}], "body": "b", "html_url": "https://github.com/o/r/issues/1",
               "url": "https://api.github.com/repos/o/r/issues/1", "updated_at": "2026-09-01T00:00:00Z"},
              {"number": 2, "title": "PR", "state": "open", "pull_request": {"url": "x"}},
              {"number": 3, "title": "Legacy", "state": "CLOSED", "url": "https://github.com/o/r/issues/3",
               "updatedAt": "2026-09-02T00:00:00Z"},
              {"title": "no number"}
            ]"#,
        )
        .unwrap();
        let parsed = parse_issue_rows(&rows);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].number, 1);
        assert_eq!(parsed[0].state, "OPEN");
        assert_eq!(parsed[0].labels, ["bug"]);
        assert_eq!(parsed[0].assignee.as_deref(), Some("alice"));
        assert_eq!(parsed[0].url, "https://github.com/o/r/issues/1");
        assert_eq!(
            parsed[0].updated_at.as_deref(),
            Some("2026-09-01T00:00:00Z")
        );
        assert_eq!(parsed[1].number, 3);
        assert_eq!(parsed[1].state, "CLOSED");
        assert_eq!(parsed[1].url, "https://github.com/o/r/issues/3");
        assert_eq!(
            parsed[1].updated_at.as_deref(),
            Some("2026-09-02T00:00:00Z")
        );
        assert!(parsed[1].body.is_none());
    }
}
