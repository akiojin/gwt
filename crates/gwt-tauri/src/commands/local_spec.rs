//! Local SPEC commands (filesystem-based)

use std::path::Path;

use gwt_core::{
    git::{
        close_local_spec, create_local_spec, delete_local_spec_artifact, get_local_spec_detail,
        list_local_spec_artifacts, list_local_specs, search_local_specs, update_local_spec,
        update_local_spec_phase, upsert_local_spec, upsert_local_spec_artifact,
        LocalSpecArtifact, LocalSpecDetail, LocalSpecMetadata, LocalSpecPhase,
        SpecIssueArtifactKind, SpecIssueSections,
    },
    StructuredError,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::commands::project::resolve_repo_path_for_project_root;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSpecSectionsData {
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
pub struct LocalSpecDetailData {
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub created_at: String,
    pub updated_at: String,
    pub sections: LocalSpecSectionsData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSpecArtifactData {
    pub spec_id: String,
    pub kind: String,
    pub artifact_name: String,
    pub content: String,
    pub updated_at: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSpecMetadataData {
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<LocalSpecSectionsData> for SpecIssueSections {
    fn from(value: LocalSpecSectionsData) -> Self {
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

impl From<LocalSpecDetail> for LocalSpecDetailData {
    fn from(value: LocalSpecDetail) -> Self {
        LocalSpecDetailData {
            id: value.id,
            title: value.title,
            status: value.status,
            phase: value.phase,
            created_at: value.created_at,
            updated_at: value.updated_at,
            sections: LocalSpecSectionsData {
                spec: value.sections.spec,
                plan: value.sections.plan,
                tasks: value.sections.tasks,
                tdd: value.sections.tdd,
                research: value.sections.research,
                data_model: value.sections.data_model,
                quickstart: value.sections.quickstart,
                contracts: value.sections.contracts,
                checklists: value.sections.checklists,
            },
        }
    }
}

impl From<LocalSpecArtifact> for LocalSpecArtifactData {
    fn from(value: LocalSpecArtifact) -> Self {
        let kind = match value.kind {
            SpecIssueArtifactKind::Doc => "doc".to_string(),
            SpecIssueArtifactKind::Contract => "contract".to_string(),
            SpecIssueArtifactKind::Checklist => "checklist".to_string(),
        };
        LocalSpecArtifactData {
            spec_id: value.spec_id,
            kind,
            artifact_name: value.artifact_name,
            content: value.content,
            updated_at: value.updated_at,
            path: value.path,
        }
    }
}

impl From<LocalSpecMetadata> for LocalSpecMetadataData {
    fn from(value: LocalSpecMetadata) -> Self {
        LocalSpecMetadataData {
            id: value.id,
            title: value.title,
            status: value.status,
            phase: value.phase,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[instrument(skip_all, fields(command = "create_local_spec_cmd", project_path))]
#[tauri::command]
pub fn create_local_spec_cmd(
    project_path: String,
    title: String,
    sections: LocalSpecSectionsData,
) -> Result<LocalSpecDetailData, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "create_local_spec_cmd"))?;
    let detail = create_local_spec(&repo_path, title.trim(), &sections.into())
        .map_err(|e| StructuredError::internal(&e, "create_local_spec_cmd"))?;
    Ok(detail.into())
}

#[instrument(skip_all, fields(command = "update_local_spec_cmd", project_path))]
#[tauri::command]
pub fn update_local_spec_cmd(
    project_path: String,
    spec_id: String,
    title: String,
    sections: LocalSpecSectionsData,
) -> Result<LocalSpecDetailData, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "update_local_spec_cmd"))?;
    let detail = update_local_spec(&repo_path, spec_id.trim(), title.trim(), &sections.into())
        .map_err(|e| StructuredError::internal(&e, "update_local_spec_cmd"))?;
    Ok(detail.into())
}

#[instrument(skip_all, fields(command = "upsert_local_spec_cmd", project_path))]
#[tauri::command]
pub fn upsert_local_spec_cmd(
    project_path: String,
    spec_id: Option<String>,
    title: String,
    sections: LocalSpecSectionsData,
) -> Result<LocalSpecDetailData, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "upsert_local_spec_cmd"))?;
    let detail = upsert_local_spec(
        &repo_path,
        spec_id.as_deref(),
        title.trim(),
        &sections.into(),
    )
    .map_err(|e| StructuredError::internal(&e, "upsert_local_spec_cmd"))?;
    Ok(detail.into())
}

#[instrument(skip_all, fields(command = "get_local_spec_detail_cmd", project_path, spec_id))]
#[tauri::command]
pub fn get_local_spec_detail_cmd(
    project_path: String,
    spec_id: String,
) -> Result<LocalSpecDetailData, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_local_spec_detail_cmd"))?;
    let detail = get_local_spec_detail(&repo_path, spec_id.trim())
        .map_err(|e| StructuredError::internal(&e, "get_local_spec_detail_cmd"))?;
    Ok(detail.into())
}

#[instrument(skip_all, fields(command = "upsert_local_spec_artifact_cmd", project_path))]
#[tauri::command]
pub fn upsert_local_spec_artifact_cmd(
    project_path: String,
    spec_id: String,
    kind: String,
    artifact_name: String,
    content: String,
) -> Result<LocalSpecArtifactData, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "upsert_local_spec_artifact_cmd"))?;
    let kind = parse_artifact_kind(&kind)
        .map_err(|e| StructuredError::internal(&e, "upsert_local_spec_artifact_cmd"))?;
    let artifact = upsert_local_spec_artifact(
        &repo_path,
        spec_id.trim(),
        kind,
        artifact_name.trim(),
        content.trim(),
    )
    .map_err(|e| StructuredError::internal(&e, "upsert_local_spec_artifact_cmd"))?;
    Ok(artifact.into())
}

#[instrument(skip_all, fields(command = "list_local_spec_artifacts_cmd", project_path))]
#[tauri::command]
pub fn list_local_spec_artifacts_cmd(
    project_path: String,
    spec_id: String,
    kind: Option<String>,
) -> Result<Vec<LocalSpecArtifactData>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_local_spec_artifacts_cmd"))?;
    let kind = match kind {
        Some(v) if !v.trim().is_empty() => Some(
            parse_artifact_kind(&v)
                .map_err(|e| StructuredError::internal(&e, "list_local_spec_artifacts_cmd"))?,
        ),
        _ => None,
    };
    let artifacts = list_local_spec_artifacts(&repo_path, spec_id.trim(), kind)
        .map_err(|e| StructuredError::internal(&e, "list_local_spec_artifacts_cmd"))?;
    Ok(artifacts.into_iter().map(Into::into).collect())
}

#[instrument(skip_all, fields(command = "delete_local_spec_artifact_cmd", project_path))]
#[tauri::command]
pub fn delete_local_spec_artifact_cmd(
    project_path: String,
    spec_id: String,
    kind: String,
    artifact_name: String,
) -> Result<bool, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "delete_local_spec_artifact_cmd"))?;
    let kind = parse_artifact_kind(&kind)
        .map_err(|e| StructuredError::internal(&e, "delete_local_spec_artifact_cmd"))?;
    delete_local_spec_artifact(&repo_path, spec_id.trim(), kind, artifact_name.trim())
        .map_err(|e| StructuredError::internal(&e, "delete_local_spec_artifact_cmd"))
}

#[instrument(skip_all, fields(command = "close_local_spec_cmd", project_path, spec_id))]
#[tauri::command]
pub fn close_local_spec_cmd(
    project_path: String,
    spec_id: String,
) -> Result<(), StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "close_local_spec_cmd"))?;
    close_local_spec(&repo_path, spec_id.trim())
        .map_err(|e| StructuredError::internal(&e, "close_local_spec_cmd"))
}

#[instrument(skip_all, fields(command = "list_local_specs_cmd", project_path))]
#[tauri::command]
pub fn list_local_specs_cmd(
    project_path: String,
) -> Result<Vec<LocalSpecMetadataData>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_local_specs_cmd"))?;
    let specs = list_local_specs(&repo_path)
        .map_err(|e| StructuredError::internal(&e, "list_local_specs_cmd"))?;
    Ok(specs.into_iter().map(Into::into).collect())
}

#[instrument(skip_all, fields(command = "search_local_specs_cmd", project_path))]
#[tauri::command]
pub fn search_local_specs_cmd(
    project_path: String,
    query: String,
) -> Result<Vec<LocalSpecMetadataData>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "search_local_specs_cmd"))?;
    let specs = search_local_specs(&repo_path, query.trim())
        .map_err(|e| StructuredError::internal(&e, "search_local_specs_cmd"))?;
    Ok(specs.into_iter().map(Into::into).collect())
}

#[instrument(skip_all, fields(command = "update_local_spec_phase_cmd", project_path))]
#[tauri::command]
pub fn update_local_spec_phase_cmd(
    project_path: String,
    spec_id: String,
    phase: String,
) -> Result<(), StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "update_local_spec_phase_cmd"))?;
    let phase = LocalSpecPhase::parse(&phase)
        .ok_or_else(|| StructuredError::internal(&format!("Invalid phase: {phase}"), "update_local_spec_phase_cmd"))?;
    update_local_spec_phase(&repo_path, spec_id.trim(), phase)
        .map_err(|e| StructuredError::internal(&e, "update_local_spec_phase_cmd"))
}

fn parse_artifact_kind(value: &str) -> Result<SpecIssueArtifactKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "doc" => Ok(SpecIssueArtifactKind::Doc),
        "contract" => Ok(SpecIssueArtifactKind::Contract),
        "checklist" => Ok(SpecIssueArtifactKind::Checklist),
        _ => Err(format!("Invalid artifact kind: {}", value.trim())),
    }
}
