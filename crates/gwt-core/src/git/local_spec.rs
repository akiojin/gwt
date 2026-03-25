//! Local filesystem-based spec operations.
//!
//! Replaces GitHub Issue-based spec management with local `specs/SPEC-{UUID8}/` directories.
//! Each SPEC directory contains `metadata.json`, `spec.md`, `plan.md`, `tasks.md`, and
//! other artifact files organized by kind (doc, contract, checklist).

use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};

// Re-export shared types from issue_spec (kept for backward compatibility)
pub use super::issue_spec::{SpecIssueArtifactKind, SpecIssueChecklist, SpecIssueSections};

/// Phase of a local SPEC lifecycle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LocalSpecPhase {
    Draft,
    Ready,
    Planned,
    ReadyForDev,
    InProgress,
    Done,
    Blocked,
}

impl LocalSpecPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Planned => "planned",
            Self::ReadyForDev => "ready-for-dev",
            Self::InProgress => "in-progress",
            Self::Done => "done",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "draft" => Some(Self::Draft),
            "ready" => Some(Self::Ready),
            "planned" => Some(Self::Planned),
            "ready-for-dev" | "ready_for_dev" => Some(Self::ReadyForDev),
            "in-progress" | "in_progress" => Some(Self::InProgress),
            "done" => Some(Self::Done),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

/// Metadata stored in `metadata.json` within each SPEC directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSpecMetadata {
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Full detail of a local SPEC including all artifact contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSpecDetail {
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub created_at: String,
    pub updated_at: String,
    pub sections: SpecIssueSections,
}

/// An artifact entry within a local SPEC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSpecArtifact {
    pub spec_id: String,
    pub kind: SpecIssueArtifactKind,
    pub artifact_name: String,
    pub content: String,
    pub updated_at: String,
    pub path: String,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn specs_dir(repo_path: &Path) -> PathBuf {
    repo_path.join("specs")
}

fn spec_dir(repo_path: &Path, spec_id: &str) -> PathBuf {
    specs_dir(repo_path).join(format!("SPEC-{spec_id}"))
}

fn metadata_path(repo_path: &Path, spec_id: &str) -> PathBuf {
    spec_dir(repo_path, spec_id).join("metadata.json")
}

fn artifact_file_path(
    repo_path: &Path,
    spec_id: &str,
    kind: SpecIssueArtifactKind,
    name: &str,
) -> PathBuf {
    let base = spec_dir(repo_path, spec_id);
    match kind {
        SpecIssueArtifactKind::Doc => base.join(name),
        SpecIssueArtifactKind::Contract => base.join("contracts").join(name),
        SpecIssueArtifactKind::Checklist => base.join("checklists").join(name),
    }
}

/// Generate the next sequential SPEC ID by finding the max existing ID + 1.
fn generate_spec_id(repo_path: &Path) -> String {
    let dir = specs_dir(repo_path);
    let max_id = fs::read_dir(&dir)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.strip_prefix("SPEC-")
                        .and_then(|s| s.parse::<u64>().ok())
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    (max_id + 1).to_string()
}

// ---------------------------------------------------------------------------
// Metadata I/O
// ---------------------------------------------------------------------------

fn read_metadata(repo_path: &Path, spec_id: &str) -> Result<LocalSpecMetadata, String> {
    let path = metadata_path(repo_path, spec_id);
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read metadata for SPEC-{spec_id}: {e}"))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse metadata for SPEC-{spec_id}: {e}"))
}

fn write_metadata(
    repo_path: &Path,
    spec_id: &str,
    metadata: &LocalSpecMetadata,
) -> Result<(), String> {
    let path = metadata_path(repo_path, spec_id);
    let content = serde_json::to_string_pretty(metadata)
        .map_err(|e| format!("Failed to serialize metadata: {e}"))?;
    fs::write(&path, format!("{content}\n"))
        .map_err(|e| format!("Failed to write metadata for SPEC-{spec_id}: {e}"))
}

fn touch_updated_at(repo_path: &Path, spec_id: &str) -> Result<(), String> {
    let mut metadata = read_metadata(repo_path, spec_id)?;
    metadata.updated_at = Utc::now().to_rfc3339();
    write_metadata(repo_path, spec_id, &metadata)
}

// ---------------------------------------------------------------------------
// Artifact I/O
// ---------------------------------------------------------------------------

fn read_artifact_content(
    repo_path: &Path,
    spec_id: &str,
    kind: SpecIssueArtifactKind,
    name: &str,
) -> String {
    let path = artifact_file_path(repo_path, spec_id, kind, name);
    fs::read_to_string(&path).unwrap_or_default()
}

fn write_artifact_content(
    repo_path: &Path,
    spec_id: &str,
    kind: SpecIssueArtifactKind,
    name: &str,
    content: &str,
) -> Result<(), String> {
    let path = artifact_file_path(repo_path, spec_id, kind, name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory for {kind:?}:{name}: {e}"))?;
    }
    let normalized = if content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{content}\n")
    };
    fs::write(&path, &normalized)
        .map_err(|e| format!("Failed to write artifact {kind:?}:{name}: {e}"))
}

/// Read all sections from local files into a `SpecIssueSections`.
fn read_all_sections(repo_path: &Path, spec_id: &str) -> SpecIssueSections {
    SpecIssueSections {
        spec: read_artifact_content(repo_path, spec_id, SpecIssueArtifactKind::Doc, "spec.md"),
        plan: read_artifact_content(repo_path, spec_id, SpecIssueArtifactKind::Doc, "plan.md"),
        tasks: read_artifact_content(repo_path, spec_id, SpecIssueArtifactKind::Doc, "tasks.md"),
        tdd: read_artifact_content(
            repo_path,
            spec_id,
            SpecIssueArtifactKind::Checklist,
            "tdd.md",
        ),
        research: read_artifact_content(
            repo_path,
            spec_id,
            SpecIssueArtifactKind::Doc,
            "research.md",
        ),
        data_model: read_artifact_content(
            repo_path,
            spec_id,
            SpecIssueArtifactKind::Doc,
            "data-model.md",
        ),
        quickstart: read_artifact_content(
            repo_path,
            spec_id,
            SpecIssueArtifactKind::Doc,
            "quickstart.md",
        ),
        contracts: read_artifact_content(repo_path, spec_id, SpecIssueArtifactKind::Contract, ""),
        checklists: read_artifact_content(repo_path, spec_id, SpecIssueArtifactKind::Checklist, ""),
    }
}

/// Write sections to local files. Only writes non-empty sections.
fn write_sections(
    repo_path: &Path,
    spec_id: &str,
    sections: &SpecIssueSections,
) -> Result<(), String> {
    let doc = SpecIssueArtifactKind::Doc;
    let checklist = SpecIssueArtifactKind::Checklist;

    let entries: &[(&str, SpecIssueArtifactKind, &str)] = &[
        (&sections.spec, doc, "spec.md"),
        (&sections.plan, doc, "plan.md"),
        (&sections.tasks, doc, "tasks.md"),
        (&sections.tdd, checklist, "tdd.md"),
        (&sections.research, doc, "research.md"),
        (&sections.data_model, doc, "data-model.md"),
        (&sections.quickstart, doc, "quickstart.md"),
    ];

    for (content, kind, name) in entries {
        if !content.trim().is_empty() {
            write_artifact_content(repo_path, spec_id, *kind, name, content)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a new local SPEC. Returns the created detail.
pub fn create_local_spec(
    repo_path: &Path,
    title: &str,
    sections: &SpecIssueSections,
) -> Result<LocalSpecDetail, String> {
    if title.trim().is_empty() {
        return Err("title is required".to_string());
    }

    let spec_id = generate_spec_id(repo_path);
    let dir = spec_dir(repo_path, &spec_id);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create SPEC directory: {e}"))?;

    let now = Utc::now().to_rfc3339();
    let metadata = LocalSpecMetadata {
        id: spec_id.clone(),
        title: title.trim().to_string(),
        status: "open".to_string(),
        phase: LocalSpecPhase::Draft.as_str().to_string(),
        created_at: now.clone(),
        updated_at: now,
    };
    write_metadata(repo_path, &spec_id, &metadata)?;
    write_sections(repo_path, &spec_id, sections)?;

    get_local_spec_detail(repo_path, &spec_id)
}

/// Update an existing local SPEC. Returns the updated detail.
pub fn update_local_spec(
    repo_path: &Path,
    spec_id: &str,
    title: &str,
    sections: &SpecIssueSections,
) -> Result<LocalSpecDetail, String> {
    if title.trim().is_empty() {
        return Err("title is required".to_string());
    }

    let mut metadata = read_metadata(repo_path, spec_id)?;
    metadata.title = title.trim().to_string();
    metadata.updated_at = Utc::now().to_rfc3339();
    write_metadata(repo_path, spec_id, &metadata)?;
    write_sections(repo_path, spec_id, sections)?;

    get_local_spec_detail(repo_path, spec_id)
}

/// Create or update a local SPEC.
pub fn upsert_local_spec(
    repo_path: &Path,
    spec_id: Option<&str>,
    title: &str,
    sections: &SpecIssueSections,
) -> Result<LocalSpecDetail, String> {
    match spec_id {
        Some(id) => update_local_spec(repo_path, id, title, sections),
        None => create_local_spec(repo_path, title, sections),
    }
}

/// Get full detail of a local SPEC.
pub fn get_local_spec_detail(repo_path: &Path, spec_id: &str) -> Result<LocalSpecDetail, String> {
    let metadata = read_metadata(repo_path, spec_id)?;
    let sections = read_all_sections(repo_path, spec_id);

    Ok(LocalSpecDetail {
        id: metadata.id,
        title: metadata.title,
        status: metadata.status,
        phase: metadata.phase,
        created_at: metadata.created_at,
        updated_at: metadata.updated_at,
        sections,
    })
}

/// List artifact files within a local SPEC, optionally filtered by kind.
pub fn list_local_spec_artifacts(
    repo_path: &Path,
    spec_id: &str,
    kind: Option<SpecIssueArtifactKind>,
) -> Result<Vec<LocalSpecArtifact>, String> {
    let dir = spec_dir(repo_path, spec_id);
    if !dir.exists() {
        return Err(format!("SPEC-{spec_id} not found"));
    }

    let mut artifacts = Vec::new();

    // Collect doc artifacts (root-level .md files)
    if kind.is_none() || kind == Some(SpecIssueArtifactKind::Doc) {
        let doc_files = [
            "spec.md",
            "plan.md",
            "tasks.md",
            "research.md",
            "data-model.md",
            "quickstart.md",
        ];
        for name in &doc_files {
            let path = dir.join(name);
            if path.exists() {
                let content = fs::read_to_string(&path).unwrap_or_default();
                if !content.trim().is_empty() {
                    artifacts.push(LocalSpecArtifact {
                        spec_id: spec_id.to_string(),
                        kind: SpecIssueArtifactKind::Doc,
                        artifact_name: name.to_string(),
                        content,
                        updated_at: file_modified_time(&path),
                        path: path.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }

    // Collect contract artifacts
    if kind.is_none() || kind == Some(SpecIssueArtifactKind::Contract) {
        collect_subdir_artifacts(
            &dir,
            "contracts",
            SpecIssueArtifactKind::Contract,
            spec_id,
            &mut artifacts,
        );
    }

    // Collect checklist artifacts
    if kind.is_none() || kind == Some(SpecIssueArtifactKind::Checklist) {
        collect_subdir_artifacts(
            &dir,
            "checklists",
            SpecIssueArtifactKind::Checklist,
            spec_id,
            &mut artifacts,
        );
    }

    artifacts.sort_by(|a, b| {
        let key_a = format!("{}:{}", a.kind_token(), a.artifact_name);
        let key_b = format!("{}:{}", b.kind_token(), b.artifact_name);
        key_a.cmp(&key_b)
    });

    Ok(artifacts)
}

/// Create or update a single artifact file.
pub fn upsert_local_spec_artifact(
    repo_path: &Path,
    spec_id: &str,
    kind: SpecIssueArtifactKind,
    name: &str,
    content: &str,
) -> Result<LocalSpecArtifact, String> {
    let dir = spec_dir(repo_path, spec_id);
    if !dir.exists() {
        return Err(format!("SPEC-{spec_id} not found"));
    }

    write_artifact_content(repo_path, spec_id, kind, name, content)?;
    touch_updated_at(repo_path, spec_id)?;

    let path = artifact_file_path(repo_path, spec_id, kind, name);
    Ok(LocalSpecArtifact {
        spec_id: spec_id.to_string(),
        kind,
        artifact_name: name.to_string(),
        content: content.to_string(),
        updated_at: file_modified_time(&path),
        path: path.to_string_lossy().to_string(),
    })
}

/// Delete an artifact file.
pub fn delete_local_spec_artifact(
    repo_path: &Path,
    spec_id: &str,
    kind: SpecIssueArtifactKind,
    name: &str,
) -> Result<bool, String> {
    let path = artifact_file_path(repo_path, spec_id, kind, name);
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path)
        .map_err(|e| format!("Failed to delete artifact {kind:?}:{name}: {e}"))?;
    touch_updated_at(repo_path, spec_id)?;
    Ok(true)
}

/// Close a local SPEC (set status to "closed").
pub fn close_local_spec(repo_path: &Path, spec_id: &str) -> Result<(), String> {
    let mut metadata = read_metadata(repo_path, spec_id)?;
    metadata.status = "closed".to_string();
    metadata.phase = LocalSpecPhase::Done.as_str().to_string();
    metadata.updated_at = Utc::now().to_rfc3339();
    write_metadata(repo_path, spec_id, &metadata)
}

/// Update the phase of a local SPEC.
pub fn update_local_spec_phase(
    repo_path: &Path,
    spec_id: &str,
    phase: LocalSpecPhase,
) -> Result<(), String> {
    let mut metadata = read_metadata(repo_path, spec_id)?;
    metadata.phase = phase.as_str().to_string();
    metadata.updated_at = Utc::now().to_rfc3339();
    write_metadata(repo_path, spec_id, &metadata)
}

/// List all local SPECs in the repository.
pub fn list_local_specs(repo_path: &Path) -> Result<Vec<LocalSpecMetadata>, String> {
    let dir = specs_dir(repo_path);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut specs = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| format!("Failed to read specs directory: {e}"))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("SPEC-") || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let spec_id = name.strip_prefix("SPEC-").unwrap_or(&name);
        match read_metadata(repo_path, spec_id) {
            Ok(meta) => specs.push(meta),
            Err(_) => continue, // Skip directories without valid metadata
        }
    }

    specs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(specs)
}

/// Simple text search across local SPECs.
pub fn search_local_specs(repo_path: &Path, query: &str) -> Result<Vec<LocalSpecMetadata>, String> {
    let all = list_local_specs(repo_path)?;
    let query_lower = query.to_lowercase();

    let results: Vec<LocalSpecMetadata> = all
        .into_iter()
        .filter(|meta| {
            if meta.title.to_lowercase().contains(&query_lower) {
                return true;
            }
            // Also search in spec.md content
            let content =
                read_artifact_content(repo_path, &meta.id, SpecIssueArtifactKind::Doc, "spec.md");
            content.to_lowercase().contains(&query_lower)
        })
        .collect();

    Ok(results)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_subdir_artifacts(
    spec_dir: &Path,
    subdir_name: &str,
    kind: SpecIssueArtifactKind,
    spec_id: &str,
    artifacts: &mut Vec<LocalSpecArtifact>,
) {
    let subdir = spec_dir.join(subdir_name);
    if !subdir.exists() || !subdir.is_dir() {
        return;
    }
    if let Ok(entries) = fs::read_dir(&subdir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let content = fs::read_to_string(&path).unwrap_or_default();
                if !content.trim().is_empty() {
                    artifacts.push(LocalSpecArtifact {
                        spec_id: spec_id.to_string(),
                        kind,
                        artifact_name: name,
                        content,
                        updated_at: file_modified_time(&path),
                        path: path.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }
}

fn file_modified_time(path: &Path) -> String {
    path.metadata()
        .and_then(|m| m.modified())
        .map(|t| {
            let dt: chrono::DateTime<Utc> = t.into();
            dt.to_rfc3339()
        })
        .unwrap_or_default()
}

impl LocalSpecArtifact {
    fn kind_token(&self) -> &'static str {
        match self.kind {
            SpecIssueArtifactKind::Doc => "doc",
            SpecIssueArtifactKind::Contract => "contract",
            SpecIssueArtifactKind::Checklist => "checklist",
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn setup() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn create_and_get_spec() {
        let tmp = setup();
        let repo = tmp.path();

        let sections = SpecIssueSections {
            spec: "# Test Spec\nSome content".to_string(),
            plan: "# Plan\nPlan content".to_string(),
            ..Default::default()
        };

        let detail = create_local_spec(repo, "Test SPEC", &sections).unwrap();
        assert_eq!(detail.title, "Test SPEC");
        assert_eq!(detail.status, "open");
        assert_eq!(detail.phase, "draft");
        assert_eq!(detail.id, "1");
        assert!(detail.sections.spec.contains("Test Spec"));
        assert!(detail.sections.plan.contains("Plan content"));
    }

    #[test]
    fn update_spec() {
        let tmp = setup();
        let repo = tmp.path();

        let sections = SpecIssueSections {
            spec: "# Original".to_string(),
            ..Default::default()
        };
        let created = create_local_spec(repo, "Original", &sections).unwrap();

        let new_sections = SpecIssueSections {
            spec: "# Updated".to_string(),
            plan: "# New Plan".to_string(),
            ..Default::default()
        };
        let updated = update_local_spec(repo, &created.id, "Updated Title", &new_sections).unwrap();

        assert_eq!(updated.title, "Updated Title");
        assert!(updated.sections.spec.contains("Updated"));
        assert!(updated.sections.plan.contains("New Plan"));
    }

    #[test]
    fn list_specs() {
        let tmp = setup();
        let repo = tmp.path();

        let s1 = SpecIssueSections {
            spec: "Spec 1".to_string(),
            ..Default::default()
        };
        let s2 = SpecIssueSections {
            spec: "Spec 2".to_string(),
            ..Default::default()
        };

        create_local_spec(repo, "First", &s1).unwrap();
        create_local_spec(repo, "Second", &s2).unwrap();

        let specs = list_local_specs(repo).unwrap();
        assert_eq!(specs.len(), 2);
    }

    #[test]
    fn upsert_and_delete_artifact() {
        let tmp = setup();
        let repo = tmp.path();

        let sections = SpecIssueSections::default();
        let created = create_local_spec(repo, "Test", &sections).unwrap();

        let artifact = upsert_local_spec_artifact(
            repo,
            &created.id,
            SpecIssueArtifactKind::Contract,
            "api.yaml",
            "openapi: 3.0\ninfo:\n  title: Test",
        )
        .unwrap();

        assert_eq!(artifact.artifact_name, "api.yaml");

        let artifacts =
            list_local_spec_artifacts(repo, &created.id, Some(SpecIssueArtifactKind::Contract))
                .unwrap();
        assert_eq!(artifacts.len(), 1);

        let deleted = delete_local_spec_artifact(
            repo,
            &created.id,
            SpecIssueArtifactKind::Contract,
            "api.yaml",
        )
        .unwrap();
        assert!(deleted);

        let artifacts =
            list_local_spec_artifacts(repo, &created.id, Some(SpecIssueArtifactKind::Contract))
                .unwrap();
        assert_eq!(artifacts.len(), 0);
    }

    #[test]
    fn close_spec() {
        let tmp = setup();
        let repo = tmp.path();

        let sections = SpecIssueSections::default();
        let created = create_local_spec(repo, "To Close", &sections).unwrap();

        close_local_spec(repo, &created.id).unwrap();

        let detail = get_local_spec_detail(repo, &created.id).unwrap();
        assert_eq!(detail.status, "closed");
        assert_eq!(detail.phase, "done");
    }

    #[test]
    fn search_specs() {
        let tmp = setup();
        let repo = tmp.path();

        let s1 = SpecIssueSections {
            spec: "Authentication flow".to_string(),
            ..Default::default()
        };
        let s2 = SpecIssueSections {
            spec: "Database migration".to_string(),
            ..Default::default()
        };

        create_local_spec(repo, "Auth Feature", &s1).unwrap();
        create_local_spec(repo, "DB Migration", &s2).unwrap();

        let results = search_local_specs(repo, "auth").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Auth Feature");
    }

    #[test]
    fn empty_title_rejected() {
        let tmp = setup();
        let repo = tmp.path();

        let result = create_local_spec(repo, "", &SpecIssueSections::default());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("title is required"));
    }
}
