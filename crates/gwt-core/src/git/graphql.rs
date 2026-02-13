//! GraphQL query builder for GitHub PR/CI status (SPEC-d6949f99)
//!
//! Provides functions to build GraphQL queries, execute them via `gh api graphql`,
//! and parse responses into `PrStatusInfo` structs.

use std::path::Path;

use super::gh_cli::gh_command;
use super::issue::resolve_repo_slug;
use super::pullrequest::{PrStatusInfo, ReviewComment, ReviewInfo, WorkflowRunInfo};

/// Build a GraphQL query to fetch PR status for multiple branches at once.
///
/// Each branch gets an aliased field `b{index}` querying the most recent OPEN PR.
pub fn build_pr_status_query(owner: &str, repo: &str, branch_names: &[String]) -> String {
    let mut fields = String::new();
    for (i, branch) in branch_names.iter().enumerate() {
        fields.push_str(&format!(
            r#"
    b{i}: pullRequests(headRefName: "{branch}", first: 1, states: OPEN) {{
      nodes {{
        number
        title
        state
        url
        mergeable
        author {{ login }}
        baseRefName
        headRefName
        labels(first: 20) {{ nodes {{ name }} }}
        assignees(first: 20) {{ nodes {{ login }} }}
        milestone {{ title }}
        closingIssuesReferences(first: 10) {{ nodes {{ number }} }}
        commits(last: 1) {{
          nodes {{
            commit {{
              statusCheckRollup {{
                contexts(first: 50) {{
                  nodes {{
                    ... on CheckRun {{
                      name
                      databaseId
                      status
                      conclusion
                    }}
                  }}
                }}
              }}
            }}
          }}
        }}
        reviews(last: 10) {{
          nodes {{
            author {{ login }}
            state
          }}
        }}
        changedFiles
        additions
        deletions
      }}
    }}"#,
            i = i,
            branch = branch,
        ));
    }

    format!(
        r#"{{ repository(owner: "{owner}", name: "{repo}") {{ {fields} }} }}"#,
        owner = owner,
        repo = repo,
        fields = fields,
    )
}

/// Parse a GraphQL response JSON into a list of (branch_name, Option<PrStatusInfo>).
pub fn parse_pr_status_response(
    json: &str,
    branch_names: &[String],
) -> Result<Vec<(String, Option<PrStatusInfo>)>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Check for GraphQL errors
    if let Some(errors) = parsed.get("errors") {
        if let Some(arr) = errors.as_array() {
            if !arr.is_empty() {
                let msg = arr
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(format!("GraphQL errors: {}", msg));
            }
        }
    }

    let repo = parsed
        .get("data")
        .and_then(|d| d.get("repository"))
        .ok_or_else(|| "Missing data.repository in response".to_string())?;

    let mut results = Vec::new();
    for (i, branch) in branch_names.iter().enumerate() {
        let alias = format!("b{}", i);
        let pr_info = repo
            .get(&alias)
            .and_then(|pr_list| pr_list.get("nodes"))
            .and_then(|nodes| nodes.as_array())
            .and_then(|arr| arr.first())
            .and_then(|node| parse_pr_node(node, branch));

        results.push((branch.clone(), pr_info));
    }

    Ok(results)
}

/// Parse a single PR node from the GraphQL response.
fn parse_pr_node(node: &serde_json::Value, _branch: &str) -> Option<PrStatusInfo> {
    let number = node.get("number")?.as_u64()?;
    let title = node.get("title")?.as_str()?.to_string();
    let state = node.get("state")?.as_str()?.to_string();
    let url = node.get("url")?.as_str()?.to_string();
    let mergeable = node
        .get("mergeable")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN")
        .to_string();
    let author = node
        .get("author")
        .and_then(|a| a.get("login"))
        .and_then(|l| l.as_str())
        .unwrap_or("unknown")
        .to_string();
    let base_branch = node
        .get("baseRefName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let head_branch = node
        .get("headRefName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let labels = node
        .get("labels")
        .and_then(|l| l.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let assignees = node
        .get("assignees")
        .and_then(|a| a.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("login").and_then(|l| l.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let milestone = node
        .get("milestone")
        .and_then(|m| m.get("title"))
        .and_then(|t| t.as_str())
        .map(String::from);

    let linked_issues = node
        .get("closingIssuesReferences")
        .and_then(|c| c.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|i| i.get("number")?.as_u64())
                .collect()
        })
        .unwrap_or_default();

    let check_suites = parse_check_suites(node);
    let reviews = parse_reviews(node);

    Some(PrStatusInfo {
        number,
        title,
        state,
        url,
        mergeable,
        author,
        base_branch,
        head_branch,
        labels,
        assignees,
        milestone,
        linked_issues,
        check_suites,
        reviews,
        review_comments: vec![],
        changed_files_count: node
            .get("changedFiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        additions: node.get("additions").and_then(|v| v.as_u64()).unwrap_or(0),
        deletions: node.get("deletions").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}

fn parse_check_suites(node: &serde_json::Value) -> Vec<WorkflowRunInfo> {
    node.get("commits")
        .and_then(|c| c.get("nodes"))
        .and_then(|n| n.as_array())
        .and_then(|arr| arr.first())
        .and_then(|commit_node| commit_node.get("commit"))
        .and_then(|commit| commit.get("statusCheckRollup"))
        .and_then(|rollup| rollup.get("contexts"))
        .and_then(|ctx| ctx.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|check| {
                    let workflow_name = check.get("name")?.as_str()?.to_string();
                    let run_id = check
                        .get("databaseId")
                        .and_then(|d| d.as_u64())
                        .unwrap_or(0);
                    let status = check
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("UNKNOWN")
                        .to_lowercase();
                    let conclusion = check
                        .get("conclusion")
                        .and_then(|c| c.as_str())
                        .map(|c| c.to_lowercase());
                    Some(WorkflowRunInfo {
                        workflow_name,
                        run_id,
                        status,
                        conclusion,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_reviews(node: &serde_json::Value) -> Vec<ReviewInfo> {
    node.get("reviews")
        .and_then(|r| r.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|review| {
                    let reviewer = review
                        .get("author")
                        .and_then(|a| a.get("login"))
                        .and_then(|l| l.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let state = review.get("state")?.as_str()?.to_string();
                    Some(ReviewInfo { reviewer, state })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Build a detailed GraphQL query for a single PR (includes reviews, review comments, file changes).
pub fn build_pr_detail_query(owner: &str, repo: &str, pr_number: u64) -> String {
    format!(
        r#"{{ repository(owner: "{owner}", name: "{repo}") {{
    pullRequest(number: {pr_number}) {{
      number
      title
      state
      url
      mergeable
      author {{ login }}
      baseRefName
      headRefName
      labels(first: 20) {{ nodes {{ name }} }}
      assignees(first: 20) {{ nodes {{ login }} }}
      milestone {{ title }}
      closingIssuesReferences(first: 10) {{ nodes {{ number }} }}
      commits(last: 1) {{
        nodes {{
          commit {{
            statusCheckRollup {{
              contexts(first: 50) {{
                nodes {{
                  ... on CheckRun {{
                    name
                    databaseId
                    status
                    conclusion
                  }}
                }}
              }}
            }}
          }}
        }}
      }}
      reviews(last: 20) {{
        nodes {{
          author {{ login }}
          state
        }}
      }}
      reviewThreads(first: 50) {{
        nodes {{
          comments(first: 10) {{
            nodes {{
              author {{ login }}
              body
              path
              line
              createdAt
            }}
          }}
        }}
      }}
      changedFiles
      additions
      deletions
    }}
  }} }}"#,
        owner = owner,
        repo = repo,
        pr_number = pr_number,
    )
}

/// Parse a detailed PR GraphQL response into PrStatusInfo.
pub fn parse_pr_detail_response(json: &str) -> Result<PrStatusInfo, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    if let Some(errors) = parsed.get("errors") {
        if let Some(arr) = errors.as_array() {
            if !arr.is_empty() {
                let msg = arr
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(format!("GraphQL errors: {}", msg));
            }
        }
    }

    let node = parsed
        .get("data")
        .and_then(|d| d.get("repository"))
        .and_then(|r| r.get("pullRequest"))
        .ok_or_else(|| "Missing data.repository.pullRequest in response".to_string())?;

    let mut info = parse_pr_node(node, "").ok_or("Failed to parse PR node")?;

    // Parse review comments from reviewThreads
    info.review_comments = parse_review_comments(node);

    Ok(info)
}

fn parse_review_comments(node: &serde_json::Value) -> Vec<ReviewComment> {
    node.get("reviewThreads")
        .and_then(|rt| rt.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|threads| {
            threads
                .iter()
                .flat_map(|thread| {
                    thread
                        .get("comments")
                        .and_then(|c| c.get("nodes"))
                        .and_then(|n| n.as_array())
                        .map(|comments| {
                            comments
                                .iter()
                                .filter_map(|comment| {
                                    let author = comment
                                        .get("author")
                                        .and_then(|a| a.get("login"))
                                        .and_then(|l| l.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    let body = comment.get("body")?.as_str()?.to_string();
                                    let file_path = comment
                                        .get("path")
                                        .and_then(|p| p.as_str())
                                        .map(String::from);
                                    let line = comment.get("line").and_then(|l| l.as_u64());
                                    let created_at = comment
                                        .get("createdAt")
                                        .and_then(|c| c.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    Some(ReviewComment {
                                        author,
                                        body,
                                        file_path,
                                        line,
                                        code_snippet: None,
                                        created_at,
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch PR statuses for multiple branches using `gh api graphql`.
pub fn fetch_pr_statuses(
    repo_path: &Path,
    branch_names: &[String],
) -> Result<Vec<(String, Option<PrStatusInfo>)>, String> {
    if branch_names.is_empty() {
        return Ok(vec![]);
    }

    let slug = resolve_repo_slug(repo_path)
        .ok_or_else(|| "Failed to resolve repository slug".to_string())?;
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid repo slug: {}", slug));
    }
    let (owner, repo) = (parts[0], parts[1]);

    let query = build_pr_status_query(owner, repo, branch_names);
    let output = gh_command()
        .args(["api", "graphql", "-f", &format!("query={}", query)])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh api graphql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api graphql failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pr_status_response(&stdout, branch_names)
}

/// Fetch detailed PR information for a single PR using `gh api graphql`.
pub fn fetch_pr_detail(repo_path: &Path, pr_number: u64) -> Result<PrStatusInfo, String> {
    let slug = resolve_repo_slug(repo_path)
        .ok_or_else(|| "Failed to resolve repository slug".to_string())?;
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid repo slug: {}", slug));
    }
    let (owner, repo) = (parts[0], parts[1]);

    let query = build_pr_detail_query(owner, repo, pr_number);
    let output = gh_command()
        .args(["api", "graphql", "-f", &format!("query={}", query)])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh api graphql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api graphql failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_pr_detail_response(&stdout)
}

/// Fetch CI workflow run log via `gh run view <run_id> --log` (T011).
pub fn gh_run_view_log(repo_path: &Path, run_id: u64) -> Result<String, String> {
    let output = gh_command()
        .args(["run", "view", &run_id.to_string(), "--log"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh run view: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh run view failed: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================================
    // T004: build_pr_status_query tests
    // ==========================================================

    #[test]
    fn test_build_pr_status_query_single_branch() {
        let query = build_pr_status_query("owner", "repo", &["feature/x".to_string()]);
        assert!(query.contains("repository(owner: \"owner\", name: \"repo\")"));
        assert!(query.contains("b0: pullRequests(headRefName: \"feature/x\""));
        assert!(query.contains("statusCheckRollup"));
    }

    #[test]
    fn test_build_pr_status_query_multiple_branches() {
        let branches = vec![
            "main".to_string(),
            "dev".to_string(),
            "feature/y".to_string(),
        ];
        let query = build_pr_status_query("org", "project", &branches);
        assert!(query.contains("b0: pullRequests(headRefName: \"main\""));
        assert!(query.contains("b1: pullRequests(headRefName: \"dev\""));
        assert!(query.contains("b2: pullRequests(headRefName: \"feature/y\""));
    }

    #[test]
    fn test_build_pr_status_query_empty_branches() {
        let query = build_pr_status_query("owner", "repo", &[]);
        assert!(query.contains("repository(owner: \"owner\", name: \"repo\")"));
        // No b0, b1, etc.
        assert!(!query.contains("b0:"));
    }

    // ==========================================================
    // T004: parse_pr_status_response tests
    // ==========================================================

    #[test]
    fn test_parse_pr_status_response_normal() {
        let json = r#"{
          "data": {
            "repository": {
              "b0": {
                "nodes": [{
                  "number": 42,
                  "title": "Add feature X",
                  "state": "OPEN",
                  "url": "https://github.com/owner/repo/pull/42",
                  "mergeable": "MERGEABLE",
                  "author": { "login": "alice" },
                  "baseRefName": "main",
                  "headRefName": "feature/x",
                  "labels": { "nodes": [{ "name": "enhancement" }] },
                  "assignees": { "nodes": [{ "login": "bob" }] },
                  "milestone": { "title": "v2.0" },
                  "closingIssuesReferences": { "nodes": [{ "number": 10 }] },
                  "commits": {
                    "nodes": [{
                      "commit": {
                        "statusCheckRollup": {
                          "contexts": {
                            "nodes": [{
                              "name": "CI",
                              "databaseId": 12345,
                              "status": "COMPLETED",
                              "conclusion": "SUCCESS"
                            }]
                          }
                        }
                      }
                    }]
                  },
                  "reviews": {
                    "nodes": [{
                      "author": { "login": "charlie" },
                      "state": "APPROVED"
                    }]
                  },
                  "changedFiles": 5,
                  "additions": 100,
                  "deletions": 20
                }]
              }
            }
          }
        }"#;

        let branches = vec!["feature/x".to_string()];
        let results = parse_pr_status_response(json, &branches).unwrap();

        assert_eq!(results.len(), 1);
        let (branch, info) = &results[0];
        assert_eq!(branch, "feature/x");

        let info = info.as_ref().unwrap();
        assert_eq!(info.number, 42);
        assert_eq!(info.title, "Add feature X");
        assert_eq!(info.state, "OPEN");
        assert_eq!(info.mergeable, "MERGEABLE");
        assert_eq!(info.author, "alice");
        assert_eq!(info.base_branch, "main");
        assert_eq!(info.head_branch, "feature/x");
        assert_eq!(info.labels, vec!["enhancement"]);
        assert_eq!(info.assignees, vec!["bob"]);
        assert_eq!(info.milestone, Some("v2.0".to_string()));
        assert_eq!(info.linked_issues, vec![10]);
        assert_eq!(info.check_suites.len(), 1);
        assert_eq!(info.check_suites[0].workflow_name, "CI");
        assert_eq!(info.check_suites[0].conclusion, Some("success".to_string()));
        assert_eq!(info.reviews.len(), 1);
        assert_eq!(info.reviews[0].reviewer, "charlie");
        assert_eq!(info.reviews[0].state, "APPROVED");
        assert_eq!(info.changed_files_count, 5);
        assert_eq!(info.additions, 100);
        assert_eq!(info.deletions, 20);
    }

    #[test]
    fn test_parse_pr_status_response_no_pr() {
        let json = r#"{
          "data": {
            "repository": {
              "b0": {
                "nodes": []
              }
            }
          }
        }"#;

        let branches = vec!["no-pr-branch".to_string()];
        let results = parse_pr_status_response(json, &branches).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "no-pr-branch");
        assert!(results[0].1.is_none());
    }

    #[test]
    fn test_parse_pr_status_response_graphql_error() {
        let json = r#"{
          "errors": [
            { "message": "Could not resolve to a Repository" }
          ]
        }"#;

        let branches = vec!["main".to_string()];
        let result = parse_pr_status_response(json, &branches);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Could not resolve to a Repository"));
    }

    #[test]
    fn test_parse_pr_status_response_invalid_json() {
        let result = parse_pr_status_response("not json", &["main".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pr_status_response_partial_fields() {
        // PR with minimal fields - no milestone, no labels, no assignees, no checks
        let json = r#"{
          "data": {
            "repository": {
              "b0": {
                "nodes": [{
                  "number": 1,
                  "title": "Minimal PR",
                  "state": "OPEN",
                  "url": "https://github.com/owner/repo/pull/1",
                  "mergeable": "UNKNOWN",
                  "author": { "login": "user" },
                  "baseRefName": "main",
                  "headRefName": "fix/bug",
                  "labels": { "nodes": [] },
                  "assignees": { "nodes": [] },
                  "milestone": null,
                  "closingIssuesReferences": { "nodes": [] },
                  "commits": { "nodes": [] },
                  "reviews": { "nodes": [] },
                  "changedFiles": 0,
                  "additions": 0,
                  "deletions": 0
                }]
              }
            }
          }
        }"#;

        let branches = vec!["fix/bug".to_string()];
        let results = parse_pr_status_response(json, &branches).unwrap();
        let info = results[0].1.as_ref().unwrap();

        assert_eq!(info.number, 1);
        assert!(info.labels.is_empty());
        assert!(info.assignees.is_empty());
        assert!(info.milestone.is_none());
        assert!(info.linked_issues.is_empty());
        assert!(info.check_suites.is_empty());
        assert!(info.reviews.is_empty());
    }

    #[test]
    fn test_parse_pr_status_response_multiple_branches() {
        let json = r#"{
          "data": {
            "repository": {
              "b0": {
                "nodes": [{
                  "number": 1,
                  "title": "PR 1",
                  "state": "OPEN",
                  "url": "https://github.com/o/r/pull/1",
                  "mergeable": "MERGEABLE",
                  "author": { "login": "a" },
                  "baseRefName": "main",
                  "headRefName": "branch-a",
                  "labels": { "nodes": [] },
                  "assignees": { "nodes": [] },
                  "milestone": null,
                  "closingIssuesReferences": { "nodes": [] },
                  "commits": { "nodes": [] },
                  "reviews": { "nodes": [] },
                  "changedFiles": 1,
                  "additions": 10,
                  "deletions": 5
                }]
              },
              "b1": {
                "nodes": []
              }
            }
          }
        }"#;

        let branches = vec!["branch-a".to_string(), "branch-b".to_string()];
        let results = parse_pr_status_response(json, &branches).unwrap();

        assert_eq!(results.len(), 2);
        assert!(results[0].1.is_some());
        assert_eq!(results[0].1.as_ref().unwrap().number, 1);
        assert!(results[1].1.is_none());
    }

    // ==========================================================
    // T006: build_pr_detail_query tests
    // ==========================================================

    #[test]
    fn test_build_pr_detail_query() {
        let query = build_pr_detail_query("owner", "repo", 42);
        assert!(query.contains("pullRequest(number: 42)"));
        assert!(query.contains("reviewThreads"));
        assert!(query.contains("changedFiles"));
        assert!(query.contains("reviews"));
    }

    // ==========================================================
    // T006: parse_pr_detail_response tests
    // ==========================================================

    #[test]
    fn test_parse_pr_detail_response_normal() {
        let json = r#"{
          "data": {
            "repository": {
              "pullRequest": {
                "number": 42,
                "title": "Detailed PR",
                "state": "OPEN",
                "url": "https://github.com/owner/repo/pull/42",
                "mergeable": "MERGEABLE",
                "author": { "login": "alice" },
                "baseRefName": "main",
                "headRefName": "feature/detail",
                "labels": { "nodes": [{ "name": "bug" }] },
                "assignees": { "nodes": [] },
                "milestone": null,
                "closingIssuesReferences": { "nodes": [] },
                "commits": { "nodes": [] },
                "reviews": {
                  "nodes": [{
                    "author": { "login": "bob" },
                    "state": "CHANGES_REQUESTED"
                  }]
                },
                "reviewThreads": {
                  "nodes": [{
                    "comments": {
                      "nodes": [{
                        "author": { "login": "bob" },
                        "body": "Fix this line",
                        "path": "src/main.rs",
                        "line": 42,
                        "createdAt": "2025-01-01T00:00:00Z"
                      }]
                    }
                  }]
                },
                "changedFiles": 3,
                "additions": 50,
                "deletions": 10
              }
            }
          }
        }"#;

        let info = parse_pr_detail_response(json).unwrap();
        assert_eq!(info.number, 42);
        assert_eq!(info.title, "Detailed PR");
        assert_eq!(info.reviews.len(), 1);
        assert_eq!(info.reviews[0].state, "CHANGES_REQUESTED");
        assert_eq!(info.review_comments.len(), 1);
        assert_eq!(info.review_comments[0].author, "bob");
        assert_eq!(info.review_comments[0].body, "Fix this line");
        assert_eq!(
            info.review_comments[0].file_path,
            Some("src/main.rs".to_string())
        );
        assert_eq!(info.review_comments[0].line, Some(42));
        assert_eq!(info.changed_files_count, 3);
    }

    #[test]
    fn test_parse_pr_detail_response_graphql_error() {
        let json = r#"{
          "errors": [{ "message": "Not found" }]
        }"#;

        let result = parse_pr_detail_response(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not found"));
    }

    #[test]
    fn test_parse_pr_detail_response_invalid_json() {
        let result = parse_pr_detail_response("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pr_detail_response_no_review_comments() {
        let json = r#"{
          "data": {
            "repository": {
              "pullRequest": {
                "number": 1,
                "title": "No comments",
                "state": "OPEN",
                "url": "https://github.com/o/r/pull/1",
                "mergeable": "UNKNOWN",
                "author": { "login": "user" },
                "baseRefName": "main",
                "headRefName": "fix/x",
                "labels": { "nodes": [] },
                "assignees": { "nodes": [] },
                "milestone": null,
                "closingIssuesReferences": { "nodes": [] },
                "commits": { "nodes": [] },
                "reviews": { "nodes": [] },
                "reviewThreads": { "nodes": [] },
                "changedFiles": 0,
                "additions": 0,
                "deletions": 0
              }
            }
          }
        }"#;

        let info = parse_pr_detail_response(json).unwrap();
        assert!(info.review_comments.is_empty());
    }
}
