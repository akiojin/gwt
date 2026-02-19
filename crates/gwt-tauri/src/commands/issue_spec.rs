//! Issue-first Spec Kit commands

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::git::{
    close_spec_issue, delete_spec_issue_artifact_comment, find_spec_issue_by_spec_id,
    get_spec_issue_detail, list_spec_issue_artifact_comments, sync_issue_to_project,
    upsert_spec_issue, upsert_spec_issue_artifact_comment, ProjectSyncResult,
    SpecIssueArtifactComment, SpecIssueArtifactKind, SpecIssueDetail, SpecIssueSections,
    SpecProjectPhase,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecIssueSectionsData {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecIssueDetailData {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub updated_at: String,
    pub spec_id: Option<String>,
    pub labels: Vec<String>,
    pub etag: String,
    pub body: String,
    pub sections: SpecIssueSectionsData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecIssueArtifactCommentData {
    pub comment_id: String,
    pub issue_number: u64,
    pub kind: String,
    pub artifact_name: String,
    pub content: String,
    pub updated_at: String,
    pub etag: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSpecIssueProjectResult {
    pub project_item_id: Option<String>,
    pub status_applied: bool,
    pub warning: Option<String>,
}

impl From<SpecIssueSectionsData> for SpecIssueSections {
    fn from(value: SpecIssueSectionsData) -> Self {
        SpecIssueSections {
            spec: value.spec,
            plan: value.plan,
            tasks: value.tasks,
            tdd: value.tdd,
            research: value.research,
            data_model: value.data_model,
            quickstart: value.quickstart,
            contracts: value.contracts,
            checklists: value.checklists,
        }
    }
}

impl From<SpecIssueSections> for SpecIssueSectionsData {
    fn from(value: SpecIssueSections) -> Self {
        SpecIssueSectionsData {
            spec: value.spec,
            plan: value.plan,
            tasks: value.tasks,
            tdd: value.tdd,
            research: value.research,
            data_model: value.data_model,
            quickstart: value.quickstart,
            contracts: value.contracts,
            checklists: value.checklists,
        }
    }
}

impl From<SpecIssueArtifactComment> for SpecIssueArtifactCommentData {
    fn from(value: SpecIssueArtifactComment) -> Self {
        let kind = match value.kind {
            SpecIssueArtifactKind::Contract => "contract".to_string(),
            SpecIssueArtifactKind::Checklist => "checklist".to_string(),
        };
        SpecIssueArtifactCommentData {
            comment_id: value.comment_id,
            issue_number: value.issue_number,
            kind,
            artifact_name: value.artifact_name,
            content: value.content,
            updated_at: value.updated_at,
            etag: value.etag,
            url: value.url,
        }
    }
}

impl From<SpecIssueDetail> for SpecIssueDetailData {
    fn from(value: SpecIssueDetail) -> Self {
        SpecIssueDetailData {
            number: value.number,
            title: value.title,
            url: value.url,
            updated_at: value.updated_at,
            spec_id: value.spec_id,
            labels: value.labels,
            etag: value.etag,
            body: value.body,
            sections: value.sections.into(),
        }
    }
}

impl From<ProjectSyncResult> for SyncSpecIssueProjectResult {
    fn from(value: ProjectSyncResult) -> Self {
        SyncSpecIssueProjectResult {
            project_item_id: value.project_item_id,
            status_applied: value.status_applied,
            warning: value.warning,
        }
    }
}

#[tauri::command]
pub fn upsert_spec_issue_cmd(
    project_path: String,
    spec_id: String,
    title: String,
    sections: SpecIssueSectionsData,
    expected_etag: Option<String>,
) -> Result<SpecIssueDetailData, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let detail = upsert_spec_issue(
        &repo_path,
        spec_id.trim(),
        title.trim(),
        &sections.into(),
        expected_etag.as_deref(),
    )?;
    Ok(detail.into())
}

#[tauri::command]
pub fn get_spec_issue_detail_cmd(
    project_path: String,
    issue_number: u64,
) -> Result<SpecIssueDetailData, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let detail = get_spec_issue_detail(&repo_path, issue_number)?;
    Ok(detail.into())
}

#[tauri::command]
pub fn find_spec_issue_by_spec_id_cmd(
    project_path: String,
    spec_id: String,
) -> Result<Option<SpecIssueDetailData>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let detail = find_spec_issue_by_spec_id(&repo_path, spec_id.trim())?;
    Ok(detail.map(Into::into))
}

#[tauri::command]
pub fn append_spec_contract_comment_cmd(
    project_path: String,
    issue_number: u64,
    contract_name: String,
    content: String,
) -> Result<(), String> {
    let _ = upsert_spec_issue_artifact_comment_cmd(
        project_path,
        issue_number,
        "contract".to_string(),
        contract_name,
        content,
        None,
    )?;
    Ok(())
}

#[tauri::command]
pub fn upsert_spec_issue_artifact_comment_cmd(
    project_path: String,
    issue_number: u64,
    kind: String,
    artifact_name: String,
    content: String,
    expected_etag: Option<String>,
) -> Result<SpecIssueArtifactCommentData, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let kind = parse_artifact_kind(&kind)?;
    let comment = upsert_spec_issue_artifact_comment(
        &repo_path,
        issue_number,
        kind,
        artifact_name.trim(),
        content.trim(),
        expected_etag.as_deref(),
    )?;
    Ok(comment.into())
}

#[tauri::command]
pub fn list_spec_issue_artifact_comments_cmd(
    project_path: String,
    issue_number: u64,
    kind: Option<String>,
) -> Result<Vec<SpecIssueArtifactCommentData>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let kind = match kind {
        Some(v) if !v.trim().is_empty() => Some(parse_artifact_kind(&v)?),
        _ => None,
    };
    let comments = list_spec_issue_artifact_comments(&repo_path, issue_number, kind)?;
    Ok(comments.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub fn delete_spec_issue_artifact_comment_cmd(
    project_path: String,
    issue_number: u64,
    kind: String,
    artifact_name: String,
    expected_etag: Option<String>,
) -> Result<bool, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let kind = parse_artifact_kind(&kind)?;
    delete_spec_issue_artifact_comment(
        &repo_path,
        issue_number,
        kind,
        artifact_name.trim(),
        expected_etag.as_deref(),
    )
}

#[tauri::command]
pub fn close_spec_issue_cmd(project_path: String, issue_number: u64) -> Result<(), String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    close_spec_issue(&repo_path, issue_number)
}

#[tauri::command]
pub fn sync_spec_issue_project_cmd(
    project_path: String,
    issue_number: u64,
    project_id: String,
    phase: String,
) -> Result<SyncSpecIssueProjectResult, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let phase = parse_phase(&phase)?;
    let result = sync_issue_to_project(&repo_path, issue_number, project_id.trim(), phase)?;
    Ok(result.into())
}

fn parse_phase(value: &str) -> Result<SpecProjectPhase, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "draft" => Ok(SpecProjectPhase::Draft),
        "ready" => Ok(SpecProjectPhase::Ready),
        "planned" => Ok(SpecProjectPhase::Planned),
        "ready-for-dev" => Ok(SpecProjectPhase::ReadyForDev),
        "in-progress" => Ok(SpecProjectPhase::InProgress),
        "done" => Ok(SpecProjectPhase::Done),
        "blocked" => Ok(SpecProjectPhase::Blocked),
        _ => Err(format!("Invalid phase: {}", value.trim())),
    }
}

fn parse_artifact_kind(value: &str) -> Result<SpecIssueArtifactKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "contract" => Ok(SpecIssueArtifactKind::Contract),
        "checklist" => Ok(SpecIssueArtifactKind::Checklist),
        _ => Err(format!("Invalid artifact kind: {}", value.trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_phase_accepts_known_values() {
        assert!(matches!(
            parse_phase("ready-for-dev").unwrap(),
            SpecProjectPhase::ReadyForDev
        ));
        assert!(matches!(
            parse_phase("in-progress").unwrap(),
            SpecProjectPhase::InProgress
        ));
    }

    #[test]
    fn parse_artifact_kind_accepts_known_values() {
        assert!(matches!(
            parse_artifact_kind("contract").unwrap(),
            SpecIssueArtifactKind::Contract
        ));
        assert!(matches!(
            parse_artifact_kind("checklist").unwrap(),
            SpecIssueArtifactKind::Checklist
        ));
    }
}
