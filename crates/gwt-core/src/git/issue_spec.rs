//! Issue-first spec operations for GitHub Issues.
//!
//! `doc:*`/`contract:*`/`checklist:*` artifact comments are preferred as the
//! canonical source of truth, with Issue body sections retained as a fallback
//! for legacy specs.

use std::{collections::HashMap, path::Path};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use super::gh_cli::{gh_command, run_gh_output_with_repair};

const SPEC_LABEL: &str = "gwt-spec";
const PROJECT_FIELD_STATUS: &str = "Status";
const PROJECT_FIELD_PHASE: &str = "Phase";
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
const DOC_SPEC: &str = "spec.md";
const DOC_PLAN: &str = "plan.md";
const DOC_TASKS: &str = "tasks.md";
const DOC_TDD: &str = "tdd.md";
const DOC_RESEARCH: &str = "research.md";
const DOC_DATA_MODEL: &str = "data-model.md";
const DOC_QUICKSTART: &str = "quickstart.md";
const CHECKLIST_TDD: &str = "tdd.md";
const CHECKLIST_ACCEPTANCE: &str = "acceptance.md";

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
    Doc,
    Contract,
    Checklist,
}

impl SpecIssueArtifactKind {
    fn token(self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::Contract => "contract",
            Self::Checklist => "checklist",
        }
    }

    fn from_token(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "doc" => Some(Self::Doc),
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

/// Create a new spec issue. Returns the created issue detail.
pub fn create_spec_issue(
    repo_path: &Path,
    title: &str,
    sections: &SpecIssueSections,
) -> Result<SpecIssueDetail, String> {
    if title.trim().is_empty() {
        return Err("title is required".to_string());
    }

    let body = render_issue_index_body("#NEW", None);

    let number = gh_issue_create(repo_path, title, &body)?;
    let body = render_issue_index_body(&format!("#{number}"), None);
    gh_issue_edit(repo_path, number, title, &body)
        .map_err(|e| format!("Issue #{number} was created, but follow-up update failed: {e}"))?;
    sync_spec_issue_artifacts(repo_path, number, sections, None)?;
    get_spec_issue_detail(repo_path, number)
        .map_err(|e| format!("Issue #{number} was created and updated, but fetch failed: {e}"))
}

/// Update an existing spec issue by issue number. Returns the updated issue detail.
pub fn update_spec_issue(
    repo_path: &Path,
    issue_number: u64,
    title: &str,
    sections: &SpecIssueSections,
    expected_etag: Option<&str>,
) -> Result<SpecIssueDetail, String> {
    if title.trim().is_empty() {
        return Err("title is required".to_string());
    }

    let existing = get_spec_issue_detail(repo_path, issue_number)?;

    check_etag(expected_etag, &existing.etag)?;

    let acceptance = load_acceptance_checklist(repo_path, issue_number, &existing.body)?;
    let body = render_issue_index_body(&format!("#{issue_number}"), Some(&existing.body));

    gh_issue_edit(repo_path, issue_number, title, &body)?;
    sync_spec_issue_artifacts(repo_path, issue_number, sections, Some(&acceptance))?;
    get_spec_issue_detail(repo_path, issue_number)
}

pub fn upsert_spec_issue(
    repo_path: &Path,
    issue_number: Option<u64>,
    title: &str,
    sections: &SpecIssueSections,
    expected_etag: Option<&str>,
) -> Result<SpecIssueDetail, String> {
    if title.trim().is_empty() {
        return Err("title is required".to_string());
    }

    if let Some(issue_number) = issue_number {
        return update_spec_issue(repo_path, issue_number, title, sections, expected_etag);
    }

    create_spec_issue(repo_path, title, sections)
}

pub fn get_spec_issue_detail(
    repo_path: &Path,
    issue_number: u64,
) -> Result<SpecIssueDetail, String> {
    let issue_number_str = issue_number.to_string();
    let output = run_gh_output_with_repair(
        repo_path,
        [
            "issue",
            "view",
            issue_number_str.as_str(),
            "--json",
            "number,title,body,updatedAt,labels,url",
        ],
    )
    .map_err(|e| format!("Failed to execute gh issue view: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue view failed: {}", stderr.trim()));
    }

    let mut detail = parse_issue_detail_json(&String::from_utf8_lossy(&output.stdout))?;
    let comments = fetch_issue_comments(repo_path, issue_number)?;
    let artifacts = comments
        .into_iter()
        .filter_map(|comment| parse_artifact_comment(issue_number, comment))
        .collect::<Vec<_>>();
    detail.sections = resolve_sections_from_artifacts_and_body(&detail.body, &artifacts);
    Ok(detail)
}

pub fn list_spec_issue_artifact_comments(
    repo_path: &Path,
    issue_number: u64,
    kind: Option<SpecIssueArtifactKind>,
) -> Result<Vec<SpecIssueArtifactComment>, String> {
    let comments = fetch_issue_comments(repo_path, issue_number)?;
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

    let comments = fetch_issue_comments(repo_path, issue_number)?;
    let body = render_artifact_comment_body(kind, artifact_name, content);
    let existing = comments
        .into_iter()
        .filter_map(|comment| parse_artifact_comment(issue_number, comment))
        .find(|artifact| artifact.kind == kind && artifact.artifact_name == artifact_name);

    if let Some(found) = existing {
        check_etag(expected_etag, &found.etag)?;
        return update_issue_comment(repo_path, issue_number, &found.comment_id, &body);
    }

    add_issue_comment(repo_path, issue_number, &body)
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
    let comments = fetch_issue_comments(repo_path, issue_number)?;
    let existing = comments
        .into_iter()
        .filter_map(|comment| parse_artifact_comment(issue_number, comment))
        .find(|artifact| artifact.kind == kind && artifact.artifact_name == artifact_name);

    let Some(found) = existing else {
        return Ok(false);
    };

    check_etag(expected_etag, &found.etag)?;

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

fn render_issue_index_body(issue_ref: &str, existing_body: Option<&str>) -> String {
    if let Some(existing_body) = existing_body {
        if existing_body.contains("## Artifact Index") {
            return replace_spec_id_marker(existing_body, issue_ref);
        }
    }

    format!(
        "<!-- GWT_SPEC_ID:{issue_ref} -->\n\n## Artifact Index\n\n- `doc:spec.md`\n- `doc:plan.md`\n- `doc:tasks.md`\n- `doc:research.md`\n- `doc:data-model.md`\n- `doc:quickstart.md`\n- `checklist:tdd.md`\n- `checklist:acceptance.md`\n- `contract:*`\n- `checklist:*`\n\n## Status\n\n- Phase: Draft\n- Clarification: Pending\n- Analysis: Pending\n\n## Links\n\n- Parent: ...\n- Related: ...\n- PRs: ...\n"
    )
}

fn replace_spec_id_marker(body: &str, issue_ref: &str) -> String {
    let mut lines = body.lines();
    let mut result = String::new();
    if let Some(first) = lines.next() {
        if first.trim_start().starts_with("<!-- GWT_SPEC_ID:") {
            result.push_str(&format!("<!-- GWT_SPEC_ID:{issue_ref} -->"));
        } else {
            result.push_str(first);
        }
    }
    for line in lines {
        result.push('\n');
        result.push_str(line);
    }
    result
}

fn load_acceptance_checklist(
    repo_path: &Path,
    issue_number: u64,
    existing_body: &str,
) -> Result<SpecIssueChecklist, String> {
    let artifacts = list_spec_issue_artifact_comments(
        repo_path,
        issue_number,
        Some(SpecIssueArtifactKind::Checklist),
    )?;
    if let Some(artifact) = artifacts.iter().find(|artifact| {
        artifact
            .artifact_name
            .eq_ignore_ascii_case(CHECKLIST_ACCEPTANCE)
    }) {
        let items = artifact
            .content
            .lines()
            .filter_map(normalize_acceptance_checklist_line)
            .collect::<Vec<_>>();
        return Ok(SpecIssueChecklist { items });
    }
    Ok(parse_acceptance_checklist_from_body(existing_body))
}

fn sync_spec_issue_artifacts(
    repo_path: &Path,
    issue_number: u64,
    sections: &SpecIssueSections,
    acceptance_checklist: Option<&SpecIssueChecklist>,
) -> Result<(), String> {
    sync_doc_artifact(repo_path, issue_number, DOC_SPEC, &sections.spec)?;
    sync_doc_artifact(repo_path, issue_number, DOC_PLAN, &sections.plan)?;
    sync_doc_artifact(repo_path, issue_number, DOC_TASKS, &sections.tasks)?;
    sync_doc_artifact(repo_path, issue_number, DOC_RESEARCH, &sections.research)?;
    sync_doc_artifact(
        repo_path,
        issue_number,
        DOC_DATA_MODEL,
        &sections.data_model,
    )?;
    sync_doc_artifact(
        repo_path,
        issue_number,
        DOC_QUICKSTART,
        &sections.quickstart,
    )?;
    sync_checklist_artifact(repo_path, issue_number, CHECKLIST_TDD, &sections.tdd)?;

    if let Some(checklist) = acceptance_checklist {
        sync_checklist_artifact(
            repo_path,
            issue_number,
            CHECKLIST_ACCEPTANCE,
            &checklist.items.join("\n"),
        )?;
    }

    Ok(())
}

fn sync_doc_artifact(
    repo_path: &Path,
    issue_number: u64,
    name: &str,
    content: &str,
) -> Result<(), String> {
    sync_optional_artifact(
        repo_path,
        issue_number,
        SpecIssueArtifactKind::Doc,
        name,
        content,
    )
}

fn sync_checklist_artifact(
    repo_path: &Path,
    issue_number: u64,
    name: &str,
    content: &str,
) -> Result<(), String> {
    sync_optional_artifact(
        repo_path,
        issue_number,
        SpecIssueArtifactKind::Checklist,
        name,
        content,
    )
}

fn sync_optional_artifact(
    repo_path: &Path,
    issue_number: u64,
    kind: SpecIssueArtifactKind,
    name: &str,
    content: &str,
) -> Result<(), String> {
    if content.trim().is_empty() {
        let _ = delete_spec_issue_artifact_comment(repo_path, issue_number, kind, name, None)?;
        return Ok(());
    }
    let _ = upsert_spec_issue_artifact_comment(repo_path, issue_number, kind, name, content, None)?;
    Ok(())
}

fn check_etag(expected: Option<&str>, actual: &str) -> Result<(), String> {
    if let Some(e) = expected {
        if !e.trim().is_empty() && e != actual {
            return Err("etag mismatch".to_string());
        }
    }
    Ok(())
}

fn gh_issue_create(repo_path: &Path, title: &str, body: &str) -> Result<u64, String> {
    let args = vec![
        "issue", "create", "--title", title, "--body", body, "--label", SPEC_LABEL,
    ];

    let output = run_gh_output_with_repair(repo_path, args)
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
) -> Result<(), String> {
    let issue_number = issue_number.to_string();
    let args = build_issue_edit_args(issue_number.as_str(), title, body);

    let output = run_gh_output_with_repair(repo_path, args)
        .map_err(|e| format!("Failed to execute gh issue edit: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue edit failed: {}", stderr.trim()));
    }
    Ok(())
}

fn build_issue_edit_args<'a>(issue_number: &'a str, title: &'a str, body: &'a str) -> Vec<&'a str> {
    vec![
        "issue",
        "edit",
        issue_number,
        "--title",
        title,
        "--body",
        body,
        "--add-label",
        SPEC_LABEL,
    ]
}

#[derive(Debug, Clone)]
struct IssueCommentNode {
    id: String,
    body: String,
    updated_at: String,
    url: Option<String>,
}

fn fetch_issue_comments(
    repo_path: &Path,
    issue_number: u64,
) -> Result<Vec<IssueCommentNode>, String> {
    let endpoint = format!(
        "repos/{}/issues/{issue_number}/comments?per_page=100",
        repo_name_with_owner(repo_path)?
    );
    let output = run_gh_output_with_repair(repo_path, ["api", endpoint.as_str()])
        .map_err(|e| format!("Failed to execute gh api issue comments: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api issue comments failed: {}", stderr.trim()));
    }
    let comments: Vec<Value> = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid issue comments JSON: {e}"))?;
    Ok(comments
        .iter()
        .filter_map(parse_issue_comment_node)
        .collect::<Vec<_>>())
}

fn parse_issue_comment_node(value: &Value) -> Option<IssueCommentNode> {
    let id = match value.get("id")? {
        Value::String(v) => v.clone(),
        Value::Number(v) => v.to_string(),
        _ => return None,
    };
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let updated_at = value
        .get("updatedAt")
        .and_then(Value::as_str)
        .or_else(|| value.get("updated_at").and_then(Value::as_str))
        .or_else(|| value.get("lastEditedAt").and_then(Value::as_str))
        .or_else(|| value.get("createdAt").and_then(Value::as_str))
        .or_else(|| value.get("created_at").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .or_else(|| value.get("html_url").and_then(Value::as_str))
        .map(str::to_string);
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

fn build_issue_comment_payload(body: &str) -> Value {
    json!({ "body": body })
}

fn run_gh_api_with_json_input(
    repo_path: &Path,
    endpoint: &str,
    method: &str,
    payload: &Value,
) -> Result<std::process::Output, String> {
    let temp_path = std::env::temp_dir().join(format!("gwt-gh-api-{}.json", Uuid::new_v4()));
    let json_bytes = serde_json::to_vec(payload)
        .map_err(|e| format!("Failed to encode gh api payload as JSON: {e}"))?;
    std::fs::write(&temp_path, json_bytes)
        .map_err(|e| format!("Failed to write temporary gh api payload file: {e}"))?;

    let output = gh_command()
        .args(["api", endpoint, "--method", method, "--input"])
        .arg(&temp_path)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to execute gh api {method} {endpoint}: {e}"))?;

    let _ = std::fs::remove_file(&temp_path);
    Ok(output)
}

fn add_issue_comment(
    repo_path: &Path,
    issue_number: u64,
    body: &str,
) -> Result<SpecIssueArtifactComment, String> {
    let endpoint = format!(
        "repos/{}/issues/{issue_number}/comments",
        repo_name_with_owner(repo_path)?
    );
    let output = run_gh_api_with_json_input(
        repo_path,
        &endpoint,
        "POST",
        &build_issue_comment_payload(body),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Issue artifact add failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid artifact add JSON: {e}"))?;
    let comment = parse_issue_comment_node(&value)
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
    let endpoint = format!(
        "repos/{}/issues/comments/{comment_id}",
        repo_name_with_owner(repo_path)?
    );
    let output = run_gh_api_with_json_input(
        repo_path,
        &endpoint,
        "PATCH",
        &build_issue_comment_payload(body),
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Issue artifact update failed: {}", stderr.trim()));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid artifact update JSON: {e}"))?;
    let comment = parse_issue_comment_node(&value)
        .ok_or_else(|| "Updated artifact comment payload invalid".to_string())?;
    parse_artifact_comment(issue_number, comment)
        .ok_or_else(|| "Failed to parse updated artifact comment".to_string())
}

fn delete_issue_comment(repo_path: &Path, comment_id: &str) -> Result<(), String> {
    let endpoint = format!(
        "repos/{}/issues/comments/{comment_id}",
        repo_name_with_owner(repo_path)?
    );
    let output =
        run_gh_output_with_repair(repo_path, ["api", endpoint.as_str(), "--method", "DELETE"])
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

    let sections = parse_sections_from_body(&body);
    let etag = build_etag(&updated_at, &body);

    Ok(SpecIssueDetail {
        number,
        title,
        url,
        updated_at,
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

fn resolve_sections_from_artifacts_and_body(
    body: &str,
    artifacts: &[SpecIssueArtifactComment],
) -> SpecIssueSections {
    let mut sections = parse_sections_from_body(body);
    let mut contract_blocks = Vec::new();
    let mut checklist_blocks = Vec::new();

    let mut sorted_artifacts = artifacts.to_vec();
    sorted_artifacts.sort_by(|a, b| {
        a.kind
            .token()
            .cmp(b.kind.token())
            .then_with(|| a.artifact_name.cmp(&b.artifact_name))
    });

    for artifact in sorted_artifacts {
        match artifact.kind {
            SpecIssueArtifactKind::Doc => match artifact.artifact_name.as_str() {
                DOC_SPEC => sections.spec = artifact.content,
                DOC_PLAN => sections.plan = artifact.content,
                DOC_TASKS => sections.tasks = artifact.content,
                DOC_TDD => sections.tdd = artifact.content,
                DOC_RESEARCH => sections.research = artifact.content,
                DOC_DATA_MODEL => sections.data_model = artifact.content,
                DOC_QUICKSTART => sections.quickstart = artifact.content,
                _ => {}
            },
            SpecIssueArtifactKind::Contract => {
                contract_blocks.push(render_named_artifact_block(
                    artifact.kind,
                    &artifact.artifact_name,
                    &artifact.content,
                ));
            }
            SpecIssueArtifactKind::Checklist => {
                if artifact.artifact_name.eq_ignore_ascii_case(CHECKLIST_TDD) {
                    sections.tdd = artifact.content;
                } else {
                    checklist_blocks.push(render_named_artifact_block(
                        artifact.kind,
                        &artifact.artifact_name,
                        &artifact.content,
                    ));
                }
            }
        }
    }

    if !contract_blocks.is_empty() {
        sections.contracts = contract_blocks.join("\n\n");
    }
    if !checklist_blocks.is_empty() {
        sections.checklists = checklist_blocks.join("\n\n");
    }

    sections
}

fn render_named_artifact_block(
    kind: SpecIssueArtifactKind,
    artifact_name: &str,
    content: &str,
) -> String {
    let trimmed_name = artifact_name.trim();
    let trimmed_content = content.trim();
    if trimmed_content.is_empty() {
        format!("### {}:{trimmed_name}", kind.token())
    } else {
        format!("### {}:{trimmed_name}\n\n{trimmed_content}", kind.token())
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

    let (field_id, option_id) = select_project_status_field_and_option(nodes, status_name)?;

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

fn select_project_status_field_and_option(
    nodes: &[Value],
    status_name: &str,
) -> Result<(String, String), String> {
    let mut has_status_like_field = false;

    for node in nodes {
        let name = node.get("name").and_then(Value::as_str).unwrap_or_default();
        if name != PROJECT_FIELD_STATUS && name != PROJECT_FIELD_PHASE {
            continue;
        }
        has_status_like_field = true;

        let Some(field_id) = node.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(options) = node.get("options").and_then(Value::as_array) else {
            continue;
        };

        for option in options {
            let opt_name = option
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if opt_name != status_name {
                continue;
            }
            let Some(option_id) = option.get("id").and_then(Value::as_str) else {
                continue;
            };
            return Ok((field_id.to_string(), option_id.to_string()));
        }
    }

    if !has_status_like_field {
        return Err("Status field not found in project".to_string());
    }

    Err(format!("Status option not found: {status_name}"))
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
    fn build_issue_edit_args_always_reapplies_spec_label() {
        let args = build_issue_edit_args("42", "title", "body");
        assert!(args.contains(&"--add-label"));
        assert!(args.contains(&SPEC_LABEL));
    }

    #[test]
    fn parse_acceptance_checklist_preserves_checked_items() {
        let body = "## Acceptance Checklist\n\n- [x] done\n- [ ] pending\n";
        let checklist = parse_acceptance_checklist_from_body(body);
        assert_eq!(
            checklist.items,
            vec!["- [x] done".to_string(), "- [ ] pending".to_string()]
        );

        let rendered = render_artifact_comment_body(
            SpecIssueArtifactKind::Checklist,
            CHECKLIST_ACCEPTANCE,
            &checklist.items.join("\n"),
        );
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
    fn parse_artifact_comment_doc_prefix() {
        let body = "doc:spec.md\n\n# Spec";
        let parsed = parse_artifact_header_and_content(body).expect("parse doc");
        assert_eq!(parsed.kind, SpecIssueArtifactKind::Doc);
        assert_eq!(parsed.artifact_name, "spec.md");
        assert_eq!(parsed.content, "# Spec");
    }

    #[test]
    fn render_artifact_comment_body_preserves_japanese_content() {
        let body = render_artifact_comment_body(
            SpecIssueArtifactKind::Doc,
            "spec.md",
            "# Feature Specification: パフォーマンスプロファイリング基盤\n\n日本語本文",
        );
        let parsed = parse_artifact_header_and_content(&body).expect("parse japanese body");
        assert_eq!(parsed.artifact_name, "spec.md");
        assert_eq!(
            parsed.content,
            "# Feature Specification: パフォーマンスプロファイリング基盤\n\n日本語本文"
        );
    }

    #[test]
    fn build_issue_comment_payload_round_trips_non_ascii_body() {
        let payload = build_issue_comment_payload("日本語の artifact コメント本文");
        let json = serde_json::to_string(&payload).expect("serialize payload");
        let round_trip: Value = serde_json::from_str(&json).expect("parse payload");
        assert_eq!(
            round_trip.get("body").and_then(Value::as_str),
            Some("日本語の artifact コメント本文")
        );
    }

    #[test]
    fn parse_issue_comment_node_supports_rest_shape() {
        let value = json!({
            "id": 12345,
            "body": "<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->\ndoc:spec.md\n\n日本語本文",
            "updated_at": "2026-03-19T00:00:00Z",
            "html_url": "https://github.com/akiojin/gwt/issues/1705#issuecomment-1"
        });
        let parsed = parse_issue_comment_node(&value).expect("parse rest comment");
        assert_eq!(parsed.id, "12345");
        assert_eq!(parsed.updated_at, "2026-03-19T00:00:00Z");
        assert_eq!(
            parsed.url.as_deref(),
            Some("https://github.com/akiojin/gwt/issues/1705#issuecomment-1")
        );
        assert!(parsed.body.contains("日本語本文"));
    }

    #[test]
    fn resolve_sections_prefers_doc_artifacts_over_body_sections() {
        let body = "## Spec\n\nlegacy spec\n\n## Plan\n\nlegacy plan\n\n## Tasks\n\nlegacy tasks\n";
        let artifacts = vec![
            SpecIssueArtifactComment {
                comment_id: "1".to_string(),
                issue_number: 42,
                kind: SpecIssueArtifactKind::Doc,
                artifact_name: "spec.md".to_string(),
                content: "artifact spec".to_string(),
                updated_at: "2026-03-18T00:00:00Z".to_string(),
                etag: "etag1".to_string(),
                url: None,
            },
            SpecIssueArtifactComment {
                comment_id: "2".to_string(),
                issue_number: 42,
                kind: SpecIssueArtifactKind::Doc,
                artifact_name: "plan.md".to_string(),
                content: "artifact plan".to_string(),
                updated_at: "2026-03-18T00:00:00Z".to_string(),
                etag: "etag2".to_string(),
                url: None,
            },
            SpecIssueArtifactComment {
                comment_id: "3".to_string(),
                issue_number: 42,
                kind: SpecIssueArtifactKind::Checklist,
                artifact_name: "tdd.md".to_string(),
                content: "artifact tdd".to_string(),
                updated_at: "2026-03-18T00:00:00Z".to_string(),
                etag: "etag3".to_string(),
                url: None,
            },
            SpecIssueArtifactComment {
                comment_id: "4".to_string(),
                issue_number: 42,
                kind: SpecIssueArtifactKind::Contract,
                artifact_name: "openapi.yaml".to_string(),
                content: "openapi: 3.1.0".to_string(),
                updated_at: "2026-03-18T00:00:00Z".to_string(),
                etag: "etag4".to_string(),
                url: None,
            },
            SpecIssueArtifactComment {
                comment_id: "5".to_string(),
                issue_number: 42,
                kind: SpecIssueArtifactKind::Checklist,
                artifact_name: "acceptance.md".to_string(),
                content: "- [ ] acceptance".to_string(),
                updated_at: "2026-03-18T00:00:00Z".to_string(),
                etag: "etag5".to_string(),
                url: None,
            },
        ];

        let sections = resolve_sections_from_artifacts_and_body(body, &artifacts);
        assert_eq!(sections.spec, "artifact spec");
        assert_eq!(sections.plan, "artifact plan");
        assert_eq!(sections.tasks, "legacy tasks");
        assert_eq!(sections.tdd, "artifact tdd");
        assert!(sections.contracts.contains("contract:openapi.yaml"));
        assert!(sections.checklists.contains("checklist:acceptance.md"));
    }

    #[test]
    fn resolve_sections_falls_back_to_body_when_doc_artifacts_are_missing() {
        let body = "## Spec\n\nlegacy spec\n\n## Data Model\n\nlegacy model\n";
        let sections = resolve_sections_from_artifacts_and_body(body, &[]);
        assert_eq!(sections.spec, "legacy spec");
        assert_eq!(sections.data_model, "legacy model");
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

    #[test]
    fn select_project_status_field_and_option_checks_next_candidate_field() {
        let nodes = vec![
            serde_json::json!({
                "id": "field-status",
                "name": "Status",
                "options": [
                    { "id": "opt-todo", "name": "Todo" }
                ]
            }),
            serde_json::json!({
                "id": "field-phase",
                "name": "Phase",
                "options": [
                    { "id": "opt-done", "name": "Done" }
                ]
            }),
        ];

        let (field_id, option_id) =
            select_project_status_field_and_option(&nodes, "Done").expect("status option");
        assert_eq!(field_id, "field-phase");
        assert_eq!(option_id, "opt-done");
    }

    #[test]
    fn select_project_status_field_and_option_returns_not_found_when_no_candidate_field() {
        let nodes = vec![serde_json::json!({
            "id": "field-priority",
            "name": "Priority",
            "options": [
                { "id": "opt-high", "name": "High" }
            ]
        })];

        let err =
            select_project_status_field_and_option(&nodes, "Done").expect_err("status field error");
        assert_eq!(err, "Status field not found in project");
    }
}
