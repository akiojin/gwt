//! Issue-first Spec Kit operations for GitHub Issues.
//!
//! These helpers keep Spec/Plan/Tasks and related artifacts in a single
//! GitHub Issue body instead of local markdown files.

use super::gh_cli::run_gh_output_with_repair;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const SPEC_LABEL: &str = "gwt-spec";
const SECTION_SPEC: &str = "Spec";
const SECTION_PLAN: &str = "Plan";
const SECTION_TASKS: &str = "Tasks";
const SECTION_TDD: &str = "TDD";
const SECTION_RESEARCH: &str = "Research";
const SECTION_DATA_MODEL: &str = "Data Model";
const SECTION_QUICKSTART: &str = "Quickstart";
const SECTION_CONTRACTS: &str = "Contracts";
const SECTION_CHECKLISTS: &str = "Checklists";
const SECTION_CHECKLIST_LEGACY: &str = "Checklist";
const SECTION_ACCEPTANCE_CHECKLIST: &str = "Acceptance Checklist";

const ARTIFACT_MARKER_PREFIX: &str = "<!-- GWT_SPEC_ARTIFACT:";
const ARTIFACT_MARKER_SUFFIX: &str = " -->";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpecIssueSections {
    pub spec: String,
    pub plan: String,
    pub tasks: String,
    pub tdd: String,
    pub research: String,
    pub data_model: String,
    pub quickstart: String,
    pub contracts: String,
    pub checklists: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpecIssueChecklist {
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpecIssueArtifactKind {
    Contract,
    Checklist,
}

impl SpecIssueArtifactKind {
    fn token(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Checklist => "checklist",
        }
    }

    fn from_token(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "contract" => Some(Self::Contract),
            "checklist" => Some(Self::Checklist),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecIssueArtifactComment {
    pub comment_id: String,
    pub issue_number: u64,
    pub kind: SpecIssueArtifactKind,
    pub artifact_name: String,
    pub content: String,
    pub updated_at: String,
    pub etag: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecIssueDetail {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub updated_at: String,
    pub spec_id: Option<String>,
    pub labels: Vec<String>,
    pub etag: String,
    pub body: String,
    pub sections: SpecIssueSections,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSyncResult {
    pub project_item_id: Option<String>,
    pub status_applied: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpecProjectPhase {
    Draft,
    Ready,
    Planned,
    ReadyForDev,
    InProgress,
    Done,
    Blocked,
}

impl SpecProjectPhase {
    fn as_status_name(self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Ready => "Ready",
            Self::Planned => "Planned",
            Self::ReadyForDev => "Ready for Dev",
            Self::InProgress => "In Progress",
            Self::Done => "Done",
            Self::Blocked => "Blocked",
        }
    }
}

pub fn upsert_spec_issue(
    repo_path: &Path,
    spec_id: &str,
    title: &str,
    sections: &SpecIssueSections,
    expected_etag: Option<&str>,
) -> Result<SpecIssueDetail, String> {
    if spec_id.trim().is_empty() {
        return Err("spec_id is required".to_string());
    }
    if title.trim().is_empty() {
        return Err("title is required".to_string());
    }

    let existing = find_spec_issue_by_spec_id(repo_path, spec_id)?;
    let checklist = existing
        .as_ref()
        .map(|issue| parse_acceptance_checklist_from_body(&issue.body))
        .unwrap_or_default();
    let body = render_issue_body(spec_id, sections, &checklist);

    if let Some(issue) = existing.as_ref() {
        if let Some(expected) = expected_etag {
            if !expected.trim().is_empty() && expected != issue.etag {
                return Err("etag mismatch".to_string());
            }
        }
        gh_issue_edit(repo_path, issue.number, title, &body, spec_id)?;
        return get_spec_issue_detail(repo_path, issue.number);
    }

    let number = gh_issue_create(repo_path, title, &body, spec_id)?;
    get_spec_issue_detail(repo_path, number)
}

pub fn get_spec_issue_detail(
    repo_path: &Path,
    issue_number: u64,
) -> Result<SpecIssueDetail, String> {
    let issue_number = issue_number.to_string();
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "issue",
            "view",
            issue_number.as_str(),
            "--json",
            "number,title,body,updatedAt,labels,url",
        ],
    )
    .map_err(|e| format!("Failed to execute gh issue view: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue view failed: {}", stderr.trim()));
    }

    parse_issue_detail_json(&String::from_utf8_lossy(&output.stdout))
}

pub fn find_spec_issue_by_spec_id(
    repo_path: &Path,
    spec_id: &str,
) -> Result<Option<SpecIssueDetail>, String> {
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "issue", "list", "--state", "all", "--label", spec_id, "--json", "number", "--limit",
            "1",
        ],
    )
    .map_err(|e| format!("Failed to execute gh issue list: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue list failed: {}", stderr.trim()));
    }

    let parsed: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid issue list JSON: {e}"))?;
    let Some(items) = parsed.as_array() else {
        return Ok(None);
    };
    let Some(number) = items
        .first()
        .and_then(|v| v.get("number"))
        .and_then(Value::as_u64)
    else {
        return Ok(None);
    };

    get_spec_issue_detail(repo_path, number).map(Some)
}

pub fn list_spec_issue_artifact_comments(
    repo_path: &Path,
    issue_number: u64,
    kind: Option<SpecIssueArtifactKind>,
) -> Result<Vec<SpecIssueArtifactComment>, String> {
    let (_, comments) = fetch_issue_node_and_comments(repo_path, issue_number)?;
    let mut artifacts = comments
        .into_iter()
        .filter_map(|comment| parse_artifact_comment(issue_number, comment))
        .collect::<Vec<_>>();
    if let Some(filter_kind) = kind {
        artifacts.retain(|artifact| artifact.kind == filter_kind);
    }
    Ok(artifacts)
}

pub fn upsert_spec_issue_artifact_comment(
    repo_path: &Path,
    issue_number: u64,
    kind: SpecIssueArtifactKind,
    artifact_name: &str,
    content: &str,
    expected_etag: Option<&str>,
) -> Result<SpecIssueArtifactComment, String> {
    let artifact_name = artifact_name.trim();
    if artifact_name.is_empty() {
        return Err("artifact_name is required".to_string());
    }
    let content = content.trim();
    if content.is_empty() {
        return Err("content is required".to_string());
    }

    let (issue_node_id, comments) = fetch_issue_node_and_comments(repo_path, issue_number)?;
    let body = render_artifact_comment_body(kind, artifact_name, content);
    let existing = comments
        .into_iter()
        .filter_map(|comment| parse_artifact_comment(issue_number, comment))
        .find(|artifact| artifact.kind == kind && artifact.artifact_name == artifact_name);

    if let Some(found) = existing {
        if let Some(expected) = expected_etag {
            if !expected.trim().is_empty() && expected != found.etag {
                return Err("etag mismatch".to_string());
            }
        }
        return update_issue_comment(repo_path, issue_number, &found.comment_id, &body);
    }

    add_issue_comment(repo_path, issue_number, &issue_node_id, &body)
}

pub fn delete_spec_issue_artifact_comment(
    repo_path: &Path,
    issue_number: u64,
    kind: SpecIssueArtifactKind,
    artifact_name: &str,
    expected_etag: Option<&str>,
) -> Result<bool, String> {
    let artifact_name = artifact_name.trim();
    if artifact_name.is_empty() {
        return Err("artifact_name is required".to_string());
    }
    let (_, comments) = fetch_issue_node_and_comments(repo_path, issue_number)?;
    let existing = comments
        .into_iter()
        .filter_map(|comment| parse_artifact_comment(issue_number, comment))
        .find(|artifact| artifact.kind == kind && artifact.artifact_name == artifact_name);

    let Some(found) = existing else {
        return Ok(false);
    };

    if let Some(expected) = expected_etag {
        if !expected.trim().is_empty() && expected != found.etag {
            return Err("etag mismatch".to_string());
        }
    }

    delete_issue_comment(repo_path, &found.comment_id)?;
    Ok(true)
}

pub fn append_contract_comment(
    repo_path: &Path,
    issue_number: u64,
    contract_name: &str,
    content: &str,
) -> Result<(), String> {
    let _ = upsert_spec_issue_artifact_comment(
        repo_path,
        issue_number,
        SpecIssueArtifactKind::Contract,
        contract_name,
        content,
        None,
    )?;
    Ok(())
}

pub fn close_spec_issue(repo_path: &Path, issue_number: u64) -> Result<(), String> {
    let issue_number = issue_number.to_string();
    let output = run_gh_output_with_repair(repo_path, ["issue", "close", issue_number.as_str()])
        .map_err(|e| format!("Failed to execute gh issue close: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue close failed: {}", stderr.trim()));
    }
    Ok(())
}

pub fn sync_issue_to_project(
    repo_path: &Path,
    issue_number: u64,
    project_id: &str,
    phase: SpecProjectPhase,
) -> Result<ProjectSyncResult, String> {
    let resolved_project_id = if project_id.trim().is_empty() {
        match resolve_default_project_id(repo_path) {
            Ok(id) => id,
            Err(err) => {
                return Ok(ProjectSyncResult {
                    project_item_id: None,
                    status_applied: false,
                    warning: Some(format!(
                        "Project ID auto-resolution failed via GraphQL: {err}"
                    )),
                });
            }
        }
    } else {
        project_id.trim().to_string()
    };

    let issue_node_id = get_issue_node_id(repo_path, issue_number)?;
    let project_item_id = ensure_project_item(repo_path, &resolved_project_id, &issue_node_id)?;
    let status_name = phase.as_status_name();
    let status_applied = match update_project_status(
        repo_path,
        &resolved_project_id,
        &project_item_id,
        status_name,
    ) {
        Ok(updated) => updated,
        Err(err) => {
            return Ok(ProjectSyncResult {
                project_item_id: Some(project_item_id),
                status_applied: false,
                warning: Some(err),
            });
        }
    };

    Ok(ProjectSyncResult {
        project_item_id: Some(project_item_id),
        status_applied,
        warning: None,
    })
}

fn resolve_default_project_id(repo_path: &Path) -> Result<String, String> {
    let repo_slug = repo_name_with_owner(repo_path)?;
    let (owner, repo_name) = split_repo_slug(&repo_slug)?;

    if let Some(project_id) = query_repository_project_id(repo_path, owner, repo_name)? {
        return Ok(project_id);
    }

    if let Some(project_id) = query_owner_project_id(repo_path, owner, &repo_slug)? {
        return Ok(project_id);
    }

    Err(format!(
        "No active GitHub Project found for repository {repo_slug}"
    ))
}

fn repo_name_with_owner(repo_path: &Path) -> Result<String, String> {
    let output = run_gh_output_with_repair(repo_path, ["repo", "view", "--json", "nameWithOwner"])
        .map_err(|e| format!("Failed to execute gh repo view: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh repo view failed: {}", stderr.trim()));
    }
    parse_repo_slug_from_repo_view_json(&output.stdout)
}

fn parse_repo_slug_from_repo_view_json(bytes: &[u8]) -> Result<String, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid gh repo view JSON: {e}"))?;
    let slug = value
        .get("nameWithOwner")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "nameWithOwner was not found in gh repo view output".to_string())?;
    Ok(slug.to_string())
}

fn split_repo_slug(slug: &str) -> Result<(&str, &str), String> {
    let Some((owner, repo_name)) = slug.split_once('/') else {
        return Err(format!("Invalid repository slug: {slug}"));
    };
    let owner = owner.trim();
    let repo_name = repo_name.trim();
    if owner.is_empty() || repo_name.is_empty() {
        return Err(format!("Invalid repository slug: {slug}"));
    }
    Ok((owner, repo_name))
}

fn query_repository_project_id(
    repo_path: &Path,
    owner: &str,
    repo_name: &str,
) -> Result<Option<String>, String> {
    let query = "query($owner:String!, $repo:String!){ repository(owner:$owner, name:$repo){ projectsV2(first:20, orderBy:{field:UPDATED_AT, direction:DESC}){ nodes { id closed } } } }";
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("repo={repo_name}"),
        ],
    )
    .map_err(|e| format!("Failed to query repository projects: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Repository project query failed: {}",
            stderr.trim()
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid repository project JSON: {e}"))?;
    let nodes = value
        .get("data")
        .and_then(|v| v.get("repository"))
        .and_then(|v| v.get("projectsV2"))
        .and_then(|v| v.get("nodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(select_project_id_from_nodes(&nodes))
}

fn query_owner_project_id(
    repo_path: &Path,
    owner: &str,
    repo_slug: &str,
) -> Result<Option<String>, String> {
    let user_query = "query($login:String!){ user(login:$login){ projectsV2(first:20, orderBy:{field:UPDATED_AT, direction:DESC}){ nodes { id closed repositories(first:50){ nodes { nameWithOwner } } } } } }";
    if let Some(project_id) =
        query_owner_projects_by_kind(repo_path, user_query, "login", owner, repo_slug)?
    {
        return Ok(Some(project_id));
    }

    let org_query = "query($login:String!){ organization(login:$login){ projectsV2(first:20, orderBy:{field:UPDATED_AT, direction:DESC}){ nodes { id closed repositories(first:50){ nodes { nameWithOwner } } } } } }";
    query_owner_projects_by_kind(repo_path, org_query, "login", owner, repo_slug)
}

fn query_owner_projects_by_kind(
    repo_path: &Path,
    query: &str,
    var_name: &str,
    var_value: &str,
    repo_slug: &str,
) -> Result<Option<String>, String> {
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("{var_name}={var_value}"),
        ],
    )
    .map_err(|e| format!("Failed to query owner projects: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Owner project query failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid owner project JSON: {e}"))?;
    Ok(select_project_id_from_owner_projects(&value, repo_slug))
}

fn select_project_id_from_nodes(nodes: &[Value]) -> Option<String> {
    for node in nodes {
        let is_closed = node.get("closed").and_then(Value::as_bool).unwrap_or(false);
        if is_closed {
            continue;
        }
        if let Some(id) = node.get("id").and_then(Value::as_str).map(str::to_string) {
            return Some(id);
        }
    }
    nodes
        .iter()
        .find_map(|node| node.get("id").and_then(Value::as_str).map(str::to_string))
}

fn select_project_id_from_owner_projects(value: &Value, repo_slug: &str) -> Option<String> {
    let owner_node = value
        .get("data")
        .and_then(|v| v.get("user").or_else(|| v.get("organization")))?;
    let nodes = owner_node
        .get("projectsV2")
        .and_then(|v| v.get("nodes"))
        .and_then(Value::as_array)?;

    for node in nodes {
        if node.get("closed").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let repos = node
            .get("repositories")
            .and_then(|v| v.get("nodes"))
            .and_then(Value::as_array);
        let linked = repos
            .map(|items| {
                items.iter().any(|repo| {
                    repo.get("nameWithOwner")
                        .and_then(Value::as_str)
                        .map(|name| name.eq_ignore_ascii_case(repo_slug))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if linked {
            if let Some(id) = node.get("id").and_then(Value::as_str).map(str::to_string) {
                return Some(id);
            }
        }
    }

    None
}

fn render_issue_body(
    spec_id: &str,
    sections: &SpecIssueSections,
    checklist: &SpecIssueChecklist,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("<!-- GWT_SPEC_ID:{} -->\n\n", spec_id));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_SPEC,
        non_empty_or_todo(&sections.spec)
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_PLAN,
        non_empty_or_todo(&sections.plan)
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_TASKS,
        non_empty_or_todo(&sections.tasks)
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_TDD,
        non_empty_or_todo(&sections.tdd)
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_RESEARCH,
        non_empty_or_todo(&sections.research)
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_DATA_MODEL,
        non_empty_or_todo(&sections.data_model)
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_QUICKSTART,
        non_empty_or_todo(&sections.quickstart)
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_CONTRACTS,
        non_empty_or_default(
            &sections.contracts,
            "Artifact files under `contracts/` are managed in issue comments with `contract:<name>` entries."
        )
    ));
    out.push_str(&format!(
        "## {}\n\n{}\n\n",
        SECTION_CHECKLISTS,
        non_empty_or_default(
            &sections.checklists,
            "Artifact files under `checklists/` are managed in issue comments with `checklist:<name>` entries."
        )
    ));
    out.push_str(&format!("## {}\n\n", SECTION_ACCEPTANCE_CHECKLIST));
    if checklist.items.is_empty() {
        out.push_str("- [ ] Add acceptance checklist\n");
    } else {
        for item in &checklist.items {
            if let Some(normalized) = normalize_acceptance_checklist_line(item) {
                out.push_str(&normalized);
                out.push('\n');
                continue;
            }
            out.push_str("- [ ] ");
            out.push_str(item.trim());
            out.push('\n');
        }
    }
    out
}

fn non_empty_or_todo(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "_TODO_".to_string()
    } else {
        trimmed.to_string()
    }
}

fn non_empty_or_default(value: &str, default_value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default_value.to_string()
    } else {
        trimmed.to_string()
    }
}

fn gh_issue_create(
    repo_path: &Path,
    title: &str,
    body: &str,
    spec_id: &str,
) -> Result<u64, String> {
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "issue", "create", "--title", title, "--body", body, "--label", SPEC_LABEL, "--label",
            spec_id,
        ],
    )
    .map_err(|e| format!("Failed to execute gh issue create: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue create failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_issue_number_from_create_output(stdout.trim())
}

fn gh_issue_edit(
    repo_path: &Path,
    issue_number: u64,
    title: &str,
    body: &str,
    spec_id: &str,
) -> Result<(), String> {
    let issue_number = issue_number.to_string();
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "issue",
            "edit",
            issue_number.as_str(),
            "--title",
            title,
            "--body",
            body,
            "--add-label",
            SPEC_LABEL,
            "--add-label",
            spec_id,
        ],
    )
    .map_err(|e| format!("Failed to execute gh issue edit: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue edit failed: {}", stderr.trim()));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct IssueCommentNode {
    id: String,
    body: String,
    updated_at: String,
    url: Option<String>,
}

fn fetch_issue_node_and_comments(
    repo_path: &Path,
    issue_number: u64,
) -> Result<(String, Vec<IssueCommentNode>), String> {
    let issue_number = issue_number.to_string();
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "issue",
            "view",
            issue_number.as_str(),
            "--json",
            "id,comments",
        ],
    )
    .map_err(|e| format!("Failed to execute gh issue view comments: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue view failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid issue comments JSON: {e}"))?;
    let issue_node_id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Issue node id missing".to_string())?;
    let comments = value
        .get("comments")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(parse_issue_comment_node)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok((issue_node_id, comments))
}

fn parse_issue_comment_node(value: &Value) -> Option<IssueCommentNode> {
    let id = value.get("id").and_then(Value::as_str)?.to_string();
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let updated_at = value
        .get("updatedAt")
        .and_then(Value::as_str)
        .or_else(|| value.get("lastEditedAt").and_then(Value::as_str))
        .or_else(|| value.get("createdAt").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let url = value.get("url").and_then(Value::as_str).map(str::to_string);
    Some(IssueCommentNode {
        id,
        body,
        updated_at,
        url,
    })
}

fn parse_artifact_comment(
    issue_number: u64,
    comment: IssueCommentNode,
) -> Option<SpecIssueArtifactComment> {
    let parsed = parse_artifact_header_and_content(&comment.body)?;
    Some(SpecIssueArtifactComment {
        comment_id: comment.id,
        issue_number,
        kind: parsed.kind,
        artifact_name: parsed.artifact_name,
        content: parsed.content.clone(),
        updated_at: comment.updated_at.clone(),
        etag: build_etag(&comment.updated_at, &parsed.content),
        url: comment.url,
    })
}

#[derive(Debug, Clone)]
struct ParsedArtifactBody {
    kind: SpecIssueArtifactKind,
    artifact_name: String,
    content: String,
}

fn parse_artifact_header_and_content(body: &str) -> Option<ParsedArtifactBody> {
    let mut lines = body.lines();
    let first_non_empty = lines.find(|line| !line.trim().is_empty())?;
    let marker = first_non_empty.trim();
    if let Some(rest) = marker.strip_prefix(ARTIFACT_MARKER_PREFIX) {
        let rest = rest.strip_suffix(ARTIFACT_MARKER_SUFFIX)?.trim();
        let (kind_token, name) = rest.split_once(':')?;
        let kind = SpecIssueArtifactKind::from_token(kind_token)?;
        let mut remaining = body
            .split_once(first_non_empty)
            .map(|(_, tail)| tail.trim_start_matches(['\n', '\r']))
            .unwrap_or_default();
        if let Some(next) = remaining.lines().next() {
            let expected_prefix = format!("{}:", kind.token());
            if next.trim().starts_with(&expected_prefix) {
                remaining = remaining
                    .split_once(next)
                    .map(|(_, tail)| tail.trim_start_matches(['\n', '\r']))
                    .unwrap_or_default();
            }
        }
        return Some(ParsedArtifactBody {
            kind,
            artifact_name: name.trim().to_string(),
            content: remaining.trim().to_string(),
        });
    }

    let (kind_token, name) = marker.split_once(':')?;
    let kind = SpecIssueArtifactKind::from_token(kind_token)?;
    let content = body
        .split_once(first_non_empty)
        .map(|(_, tail)| tail.trim_start_matches(['\n', '\r']).trim().to_string())
        .unwrap_or_default();
    Some(ParsedArtifactBody {
        kind,
        artifact_name: name.trim().to_string(),
        content,
    })
}

fn render_artifact_comment_body(
    kind: SpecIssueArtifactKind,
    artifact_name: &str,
    content: &str,
) -> String {
    let name = artifact_name.trim();
    let payload = content.trim();
    format!(
        "{ARTIFACT_MARKER_PREFIX}{}:{}{ARTIFACT_MARKER_SUFFIX}\n{}:{}\n\n{}",
        kind.token(),
        name,
        kind.token(),
        name,
        payload
    )
}

fn add_issue_comment(
    repo_path: &Path,
    issue_number: u64,
    issue_node_id: &str,
    body: &str,
) -> Result<SpecIssueArtifactComment, String> {
    let query = "mutation($subject:ID!, $body:String!){ addComment(input:{subjectId:$subject, body:$body}) { commentEdge { node { id body updatedAt url } } } }";
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("subject={issue_node_id}"),
            "-F",
            &format!("body={body}"),
        ],
    )
    .map_err(|e| format!("Failed to add issue artifact comment: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Issue artifact add failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid artifact add JSON: {e}"))?;
    let node = value
        .get("data")
        .and_then(|v| v.get("addComment"))
        .and_then(|v| v.get("commentEdge"))
        .and_then(|v| v.get("node"))
        .cloned()
        .ok_or_else(|| "Added artifact comment payload missing".to_string())?;
    let comment = parse_issue_comment_node(&node)
        .ok_or_else(|| "Added artifact comment invalid".to_string())?;
    parse_artifact_comment(issue_number, comment)
        .ok_or_else(|| "Failed to parse added artifact comment".to_string())
}

fn update_issue_comment(
    repo_path: &Path,
    issue_number: u64,
    comment_id: &str,
    body: &str,
) -> Result<SpecIssueArtifactComment, String> {
    let query = "mutation($id:ID!, $body:String!){ updateIssueComment(input:{id:$id, body:$body}) { issueComment { id body updatedAt url } } }";
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("id={comment_id}"),
            "-F",
            &format!("body={body}"),
        ],
    )
    .map_err(|e| format!("Failed to update issue artifact comment: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Issue artifact update failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid artifact update JSON: {e}"))?;
    let node = value
        .get("data")
        .and_then(|v| v.get("updateIssueComment"))
        .and_then(|v| v.get("issueComment"))
        .cloned()
        .ok_or_else(|| "Updated artifact comment payload missing".to_string())?;
    let comment = parse_issue_comment_node(&node)
        .ok_or_else(|| "Updated artifact comment payload invalid".to_string())?;
    parse_artifact_comment(issue_number, comment)
        .ok_or_else(|| "Failed to parse updated artifact comment".to_string())
}

fn delete_issue_comment(repo_path: &Path, comment_id: &str) -> Result<(), String> {
    let query = "mutation($id:ID!){ deleteIssueComment(input:{id:$id}) { clientMutationId } }";
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("id={comment_id}"),
        ],
    )
    .map_err(|e| format!("Failed to delete issue artifact comment: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Issue artifact delete failed: {}", stderr.trim()));
    }
    Ok(())
}

fn parse_issue_detail_json(json: &str) -> Result<SpecIssueDetail, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse issue detail JSON: {e}"))?;

    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Issue number missing".to_string())?;
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let updated_at = value
        .get("updatedAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let labels = value
        .get("labels")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|label| {
                    label
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let spec_id = labels
        .iter()
        .find(|label| {
            label.len() == 13
                && label.starts_with("SPEC-")
                && label[5..].chars().all(|c| c.is_ascii_hexdigit())
        })
        .cloned();

    let sections = parse_sections_from_body(&body);
    let etag = build_etag(&updated_at, &body);

    Ok(SpecIssueDetail {
        number,
        title,
        url,
        updated_at,
        spec_id,
        labels,
        etag,
        body,
        sections,
    })
}

fn parse_sections_from_body(body: &str) -> SpecIssueSections {
    let sections = split_markdown_sections(body);
    SpecIssueSections {
        spec: sections.get(SECTION_SPEC).cloned().unwrap_or_default(),
        plan: sections.get(SECTION_PLAN).cloned().unwrap_or_default(),
        tasks: sections.get(SECTION_TASKS).cloned().unwrap_or_default(),
        tdd: sections.get(SECTION_TDD).cloned().unwrap_or_default(),
        research: sections.get(SECTION_RESEARCH).cloned().unwrap_or_default(),
        data_model: sections
            .get(SECTION_DATA_MODEL)
            .cloned()
            .unwrap_or_default(),
        quickstart: sections
            .get(SECTION_QUICKSTART)
            .cloned()
            .unwrap_or_default(),
        contracts: sections.get(SECTION_CONTRACTS).cloned().unwrap_or_default(),
        checklists: sections
            .get(SECTION_CHECKLISTS)
            .or_else(|| sections.get(SECTION_CHECKLIST_LEGACY))
            .cloned()
            .unwrap_or_default(),
    }
}

fn split_markdown_sections(body: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current: Option<String> = None;
    let mut lines: Vec<String> = Vec::new();

    let flush = |current: &mut Option<String>,
                 lines: &mut Vec<String>,
                 map: &mut HashMap<String, String>| {
        if let Some(key) = current.take() {
            let content = lines.join("\n").trim().to_string();
            map.insert(key, content);
        }
        lines.clear();
    };

    for line in body.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            flush(&mut current, &mut lines, &mut map);
            current = Some(title.trim().to_string());
            continue;
        }
        if current.is_some() {
            lines.push(line.to_string());
        }
    }
    flush(&mut current, &mut lines, &mut map);
    map
}

fn parse_acceptance_checklist_from_body(body: &str) -> SpecIssueChecklist {
    let sections = split_markdown_sections(body);
    let Some(content) = sections.get(SECTION_ACCEPTANCE_CHECKLIST) else {
        return SpecIssueChecklist::default();
    };

    let items = content
        .lines()
        .filter_map(normalize_acceptance_checklist_line)
        .collect::<Vec<String>>();

    if items.is_empty() {
        SpecIssueChecklist::default()
    } else {
        SpecIssueChecklist { items }
    }
}

fn normalize_acceptance_checklist_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed
        .strip_prefix("- [")
        .or_else(|| trimmed.strip_prefix("* ["))?;
    let state = rest.chars().next()?;
    if !matches!(state, ' ' | 'x' | 'X') {
        return None;
    }
    let text = rest.get(1..)?.strip_prefix(']')?.trim();
    if text.is_empty() {
        return None;
    }
    let checked = matches!(state, 'x' | 'X');
    let state_mark = if checked { "x" } else { " " };
    Some(format!("- [{state_mark}] {text}"))
}

fn build_etag(updated_at: &str, body: &str) -> String {
    format!("{}:{}", updated_at.trim(), body.len())
}

fn parse_issue_number_from_create_output(output: &str) -> Result<u64, String> {
    for token in output.split_whitespace().rev() {
        let trimmed = token
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
            .trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let tail = trimmed.rsplit('/').next().unwrap_or(trimmed);
        let tail = tail.split(['?', '#']).next().unwrap_or(tail);
        if let Ok(number) = tail.parse::<u64>() {
            return Ok(number);
        }
    }
    Err(format!(
        "Failed to parse issue number from output: {output}"
    ))
}

fn get_issue_node_id(repo_path: &Path, issue_number: u64) -> Result<String, String> {
    let issue_number = issue_number.to_string();
    let output = run_gh_output_with_repair(
        repo_path,
        ["issue", "view", issue_number.as_str(), "--json", "id"],
    )
    .map_err(|e| format!("Failed to execute gh issue view for node id: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue view failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid issue node JSON: {e}"))?;
    value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Issue node id not found".to_string())
}

fn ensure_project_item(
    repo_path: &Path,
    project_id: &str,
    issue_node_id: &str,
) -> Result<String, String> {
    // Try to add first.
    let add_query = "mutation($project:ID!, $content:ID!){ addProjectV2ItemById(input:{projectId:$project, contentId:$content}) { item { id } } }";
    if let Ok(item_id) = run_graphql_item_add(repo_path, add_query, project_id, issue_node_id) {
        return Ok(item_id);
    }

    // If already added, look up existing item id.
    let list_query = "query($project:ID!){ node(id:$project){ ... on ProjectV2 { items(first:100){ nodes { id content { ... on Issue { id } } } } } } }";
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={list_query}"),
            "-F",
            &format!("project={project_id}"),
        ],
    )
    .map_err(|e| format!("Failed to query project items: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Project item query failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid project item JSON: {e}"))?;
    let nodes = value
        .get("data")
        .and_then(|v| v.get("node"))
        .and_then(|v| v.get("items"))
        .and_then(|v| v.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| "Project items not found".to_string())?;
    for node in nodes {
        let content_id = node
            .get("content")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if content_id == issue_node_id {
            if let Some(item_id) = node.get("id").and_then(Value::as_str) {
                return Ok(item_id.to_string());
            }
        }
    }
    Err("Failed to resolve project item id".to_string())
}

fn run_graphql_item_add(
    repo_path: &Path,
    query: &str,
    project_id: &str,
    issue_node_id: &str,
) -> Result<String, String> {
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("project={project_id}"),
            "-F",
            &format!("content={issue_node_id}"),
        ],
    )
    .map_err(|e| format!("Failed to add project item: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Project item add failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid project add JSON: {e}"))?;
    value
        .get("data")
        .and_then(|v| v.get("addProjectV2ItemById"))
        .and_then(|v| v.get("item"))
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Project item id missing".to_string())
}

fn update_project_status(
    repo_path: &Path,
    project_id: &str,
    project_item_id: &str,
    status_name: &str,
) -> Result<bool, String> {
    let query = "query($project:ID!){ node(id:$project){ ... on ProjectV2 { fields(first:100){ nodes { ... on ProjectV2SingleSelectField { id name options { id name } } } } } } }";
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-F",
            &format!("project={project_id}"),
        ],
    )
    .map_err(|e| format!("Failed to query project fields: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Project field query failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid project field JSON: {e}"))?;
    let nodes = value
        .get("data")
        .and_then(|v| v.get("node"))
        .and_then(|v| v.get("fields"))
        .and_then(|v| v.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| "Project fields not found".to_string())?;

    let mut field_id: Option<String> = None;
    let mut option_id: Option<String> = None;

    for node in nodes {
        let name = node.get("name").and_then(Value::as_str).unwrap_or_default();
        if name != "Status" {
            continue;
        }
        field_id = node.get("id").and_then(Value::as_str).map(str::to_string);
        if let Some(options) = node.get("options").and_then(Value::as_array) {
            for option in options {
                let opt_name = option
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if opt_name == status_name {
                    option_id = option.get("id").and_then(Value::as_str).map(str::to_string);
                    break;
                }
            }
        }
        break;
    }

    let Some(field_id) = field_id else {
        return Err("Status field not found in project".to_string());
    };
    let Some(option_id) = option_id else {
        return Err(format!("Status option not found: {status_name}"));
    };

    let mutation = "mutation($project:ID!, $item:ID!, $field:ID!, $option:String!){ updateProjectV2ItemFieldValue(input:{projectId:$project, itemId:$item, fieldId:$field, value:{ singleSelectOptionId:$option }}) { projectV2Item { id } } }";
    let update_output = run_gh_output_with_repair(
        repo_path,
        [
            "api",
            "graphql",
            "-f",
            &format!("query={mutation}"),
            "-F",
            &format!("project={project_id}"),
            "-F",
            &format!("item={project_item_id}"),
            "-F",
            &format!("field={field_id}"),
            "-F",
            &format!("option={option_id}"),
        ],
    )
    .map_err(|e| format!("Failed to update project status: {e}"))?;
    if !update_output.status.success() {
        let stderr = String::from_utf8_lossy(&update_output.stderr);
        return Err(format!("Project status update failed: {}", stderr.trim()));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_issue_number_from_url_output() {
        let n =
            parse_issue_number_from_create_output("https://github.com/org/repo/issues/42").unwrap();
        assert_eq!(n, 42);
    }

    #[test]
    fn parse_issue_number_prefers_tail_segment() {
        let n = parse_issue_number_from_create_output("https://github.com/123/repo/issues/42")
            .expect("parse issue number");
        assert_eq!(n, 42);
    }

    #[test]
    fn parse_acceptance_checklist_preserves_checked_items() {
        let body = "## Acceptance Checklist\n\n- [x] done\n- [ ] pending\n";
        let checklist = parse_acceptance_checklist_from_body(body);
        assert_eq!(
            checklist.items,
            vec!["- [x] done".to_string(), "- [ ] pending".to_string()]
        );

        let rendered =
            render_issue_body("SPEC-1234abcd", &SpecIssueSections::default(), &checklist);
        assert!(rendered.contains("- [x] done"));
        assert!(rendered.contains("- [ ] pending"));
    }

    #[test]
    fn parse_sections_extracts_known_headers() {
        let body = "## Spec\n\nA\n\n## Plan\n\nB\n\n## Tasks\n\nC\n\n## TDD\n\nD\n";
        let sections = parse_sections_from_body(body);
        assert_eq!(sections.spec, "A");
        assert_eq!(sections.plan, "B");
        assert_eq!(sections.tasks, "C");
        assert_eq!(sections.tdd, "D");
    }

    #[test]
    fn build_etag_uses_timestamp_and_len() {
        let etag = build_etag("2026-01-01T00:00:00Z", "abc");
        assert_eq!(etag, "2026-01-01T00:00:00Z:3");
    }

    #[test]
    fn parse_sections_supports_legacy_checklist_header() {
        let body = "## Checklist\n\n- [ ] one";
        let sections = parse_sections_from_body(body);
        assert_eq!(sections.checklists, "- [ ] one");
    }

    #[test]
    fn parse_artifact_comment_marker_round_trip() {
        let body =
            render_artifact_comment_body(SpecIssueArtifactKind::Contract, "openapi.md", "hello");
        let parsed = parse_artifact_header_and_content(&body).expect("parse marker");
        assert_eq!(parsed.kind, SpecIssueArtifactKind::Contract);
        assert_eq!(parsed.artifact_name, "openapi.md");
        assert_eq!(parsed.content, "hello");
    }

    #[test]
    fn parse_artifact_comment_legacy_prefix() {
        let body = "checklist:requirements.md\n\n- [ ] item";
        let parsed = parse_artifact_header_and_content(body).expect("parse legacy");
        assert_eq!(parsed.kind, SpecIssueArtifactKind::Checklist);
        assert_eq!(parsed.artifact_name, "requirements.md");
        assert_eq!(parsed.content, "- [ ] item");
    }

    #[test]
    fn parse_repo_slug_from_repo_view_json_extracts_name_with_owner() {
        let slug = parse_repo_slug_from_repo_view_json(br#"{"nameWithOwner":"akiojin/gwt"}"#)
            .expect("parse slug");
        assert_eq!(slug, "akiojin/gwt");
    }

    #[test]
    fn select_project_id_from_nodes_prefers_open_project() {
        let nodes = vec![
            serde_json::json!({ "id": "PVT_closed", "closed": true }),
            serde_json::json!({ "id": "PVT_open", "closed": false }),
        ];
        let project_id = select_project_id_from_nodes(&nodes).expect("project id");
        assert_eq!(project_id, "PVT_open");
    }

    #[test]
    fn select_project_id_from_owner_projects_filters_by_repo_link() {
        let payload = serde_json::json!({
            "data": {
                "user": {
                    "projectsV2": {
                        "nodes": [
                            {
                                "id": "PVT_other",
                                "closed": false,
                                "repositories": {
                                    "nodes": [
                                        { "nameWithOwner": "akiojin/other" }
                                    ]
                                }
                            },
                            {
                                "id": "PVT_match",
                                "closed": false,
                                "repositories": {
                                    "nodes": [
                                        { "nameWithOwner": "akiojin/gwt" }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        });

        let project_id =
            select_project_id_from_owner_projects(&payload, "akiojin/gwt").expect("project id");
        assert_eq!(project_id, "PVT_match");
    }
}
