//! Paged REST reads through `gh api` (SPEC #4093 FR-002).
//!
//! `gh pr list` / `gh issue list` run on GraphQL, whose hourly budget is
//! charged in points that grow with the page size, so one `--limit 999` list
//! can burn hundreds of points. The REST `core` budget is charged per request
//! regardless of size (one page of 100 rows = one request), and it is a
//! separate 5,000/h pool. Every Issue Monitor hot path reads through here so
//! its cost per scan is a bounded number of REST requests and zero GraphQL.

use serde_json::Value;

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
pub fn read_pages_with<F>(endpoint: &str, mut fetch: F) -> Result<RestPages, String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    let mut rows = Vec::new();
    let mut requests = 0;
    let mut next = Some(page_endpoint(endpoint, 1));
    while let Some(path) = next.take() {
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
        if requests == REST_MAX_PAGES_PER_READ {
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
