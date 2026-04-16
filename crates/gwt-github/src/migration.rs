//! SPEC-12 migration support: port local `specs/SPEC-N/` directories into
//! GitHub Issues using the hybrid body/comment storage layout.
//!
//! This is the Rust replacement for the one-shot `tasks/migrate_specs_v2.py`
//! script that was used for the first real-world migration. Keeping the
//! logic in the crate means any downstream project can reuse it via the
//! `gwt issue migrate-specs` subcommand without shipping an external Python
//! file.
//!
//! The migration pipeline has three entry points:
//!
//! - [`plan`]: scan a specs directory and produce a [`MigrationReport`]
//!   describing, for each SPEC, the target title, labels, and expected
//!   routing. Pure — it never touches the network.
//! - [`execute`]: run the plan against an [`IssueClient`], creating one
//!   Issue per SPEC and writing the resulting snapshots into the local
//!   [`Cache`]. Uses [`crate::SpecOps::create_spec`] under the hood to keep
//!   the "cache-mediated one-way flow" invariant intact.
//!
//! The filesystem layout assumed here mirrors what existed before SPEC-12:
//!
//! ```text
//! specs/
//!   SPEC-1/
//!     metadata.json      # { id, title, status, phase }
//!     spec.md
//!     plan.md            # optional
//!     tasks.md           # optional
//!     research.md        # optional
//!     data-model.md      # optional
//!     quickstart.md      # optional
//!     tdd.md             # optional
//!     contracts/*.md     # optional
//!     checklists/*.md    # optional
//!   SPEC-2/
//!   ...
//! ```

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    cache::Cache, client::IssueClient, sections::SectionName, spec_ops::SpecOps, SpecOpsError,
};

/// Mapping of legacy `status` field values in `metadata.json` to the canonical
/// `phase/*` labels used by SPEC-12.
fn map_status_to_phase(status: &str) -> &'static str {
    match status.to_ascii_lowercase().as_str() {
        "draft" | "specify" => "phase/draft",
        "planning" | "plan" => "phase/planning",
        "in-progress" | "implementation" => "phase/implementation",
        "review" => "phase/review",
        "closed" | "done" => "phase/done",
        _ => "phase/draft",
    }
}

/// Canonical set of section file names read from a legacy SPEC directory.
const SECTION_FILES: &[(&str, &str)] = &[
    ("spec", "spec.md"),
    ("plan", "plan.md"),
    ("tasks", "tasks.md"),
    ("research", "research.md"),
    ("data-model", "data-model.md"),
    ("quickstart", "quickstart.md"),
    ("tdd", "tdd.md"),
];

/// Single SPEC row in a migration plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    /// Source directory name (e.g. `"SPEC-3"`).
    pub source: String,
    /// Full path to the source directory.
    pub source_path: PathBuf,
    /// Title that will be submitted to GitHub.
    pub title: String,
    /// phase/* label derived from metadata.json status.
    pub phase_label: String,
    /// Raw section contents to be uploaded.
    pub sections: BTreeMap<SectionName, String>,
}

/// Output of [`plan`].
#[derive(Debug, Clone, Default)]
pub struct MigrationReport {
    pub entries: Vec<PlanEntry>,
    pub skipped: Vec<(String, String)>, // (source_name, reason)
}

/// Errors reported by migration operations.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("metadata.json parse error in {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    SpecOps(#[from] SpecOpsError),
}

/// Metadata shape we expect to find in `specs/SPEC-*/metadata.json`.
#[derive(Debug, Deserialize)]
struct LegacyMetadata {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Scan `specs_dir` and produce a plan without touching the network.
pub fn plan(specs_dir: &Path) -> Result<MigrationReport, MigrationError> {
    let mut report = MigrationReport::default();
    let read = fs::read_dir(specs_dir).map_err(|e| MigrationError::Io {
        path: specs_dir.to_path_buf(),
        source: e,
    })?;

    let mut dirs: Vec<PathBuf> = read
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.starts_with("SPEC-"))
                .unwrap_or(false)
                && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
        })
        .map(|entry| entry.path())
        .collect();

    // Deterministic ordering by the numeric tail of the SPEC directory name.
    dirs.sort_by_key(|path| {
        path.file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("SPEC-"))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });

    for dir in dirs {
        let source = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let meta_path = dir.join("metadata.json");
        if !meta_path.is_file() {
            report
                .skipped
                .push((source, "metadata.json missing".to_string()));
            continue;
        }
        let meta_bytes = fs::read(&meta_path).map_err(|e| MigrationError::Io {
            path: meta_path.clone(),
            source: e,
        })?;
        let meta: LegacyMetadata =
            serde_json::from_slice(&meta_bytes).map_err(|e| MigrationError::Metadata {
                path: meta_path.clone(),
                source: e,
            })?;

        let raw_title = meta.title.clone().unwrap_or_else(|| source.clone());
        let title = raw_title
            .strip_prefix("gwt-spec: ")
            .map(String::from)
            .unwrap_or(raw_title);
        let phase_label = map_status_to_phase(meta.status.as_deref().unwrap_or("draft"));

        let mut sections: BTreeMap<SectionName, String> = BTreeMap::new();
        for (name, filename) in SECTION_FILES {
            let path = dir.join(filename);
            if path.is_file() {
                let content = fs::read_to_string(&path).map_err(|e| MigrationError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                let trimmed = content.trim_end().to_string();
                if !trimmed.is_empty() {
                    sections.insert(SectionName((*name).to_string()), trimmed);
                }
            }
        }

        // Scan contracts/ and checklists/ subdirectories (each markdown file
        // becomes a section named `contract/<filename>` or
        // `checklist/<filename>`).
        for (subdir, prefix) in [("contracts", "contract"), ("checklists", "checklist")] {
            let sub_path = dir.join(subdir);
            if !sub_path.is_dir() {
                continue;
            }
            let sub_read = fs::read_dir(&sub_path).map_err(|e| MigrationError::Io {
                path: sub_path.clone(),
                source: e,
            })?;
            let mut files: Vec<PathBuf> = sub_read.flatten().map(|e| e.path()).collect();
            files.sort();
            for file in files {
                if file.is_file() {
                    let fname = file
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let content = fs::read_to_string(&file).map_err(|e| MigrationError::Io {
                        path: file.clone(),
                        source: e,
                    })?;
                    let trimmed = content.trim_end().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }
                    sections.insert(SectionName(format!("{prefix}/{fname}")), trimmed);
                }
            }
        }

        report.entries.push(PlanEntry {
            source: source.clone(),
            source_path: dir,
            title: format!("gwt-spec: {title}"),
            phase_label: phase_label.to_string(),
            sections,
        });
    }

    Ok(report)
}

/// Execute a previously-computed plan against the given client and cache.
///
/// For each [`PlanEntry`] we call [`SpecOps::create_spec`], which routes
/// sections through the hybrid body/comment layout, creates the Issue, and
/// persists the result in the cache. Errors for individual SPECs are
/// surfaced as `Err` on the returned `Vec<Result<...>>` so the caller can
/// decide whether to continue with a partial migration.
pub fn execute<C: IssueClient>(
    report: &MigrationReport,
    client: C,
    cache: Cache,
) -> Vec<Result<MigratedSpec, (String, SpecOpsError)>> {
    let ops = SpecOps::new(client, cache);
    let mut results = Vec::with_capacity(report.entries.len());
    for entry in &report.entries {
        match ops.create_spec(
            &entry.title,
            entry.sections.clone(),
            std::slice::from_ref(&entry.phase_label),
        ) {
            Ok(snapshot) => results.push(Ok(MigratedSpec {
                source: entry.source.clone(),
                new_issue_number: snapshot.number.0,
                phase_label: entry.phase_label.clone(),
            })),
            Err(e) => results.push(Err((entry.source.clone(), e))),
        }
    }
    results
}

/// Success record for a single migrated SPEC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigratedSpec {
    pub source: String,
    pub new_issue_number: u64,
    pub phase_label: String,
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn write_spec(
        root: &Path,
        name: &str,
        title: &str,
        status: &str,
        files: &[(&str, &str)],
    ) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let metadata = serde_json::json!({
            "id": name.trim_start_matches("SPEC-"),
            "title": title,
            "status": status,
        });
        fs::write(dir.join("metadata.json"), metadata.to_string()).unwrap();
        for (filename, content) in files {
            fs::write(dir.join(filename), content).unwrap();
        }
        dir
    }

    #[test]
    fn plan_reads_basic_spec_directory() {
        let tmp = TempDir::new().unwrap();
        let specs = tmp.path().join("specs");
        fs::create_dir_all(&specs).unwrap();
        write_spec(
            &specs,
            "SPEC-1",
            "gwt-spec: Terminal emulation",
            "done",
            &[("spec.md", "# spec\nbody\n"), ("tasks.md", "- [ ] T-001\n")],
        );
        write_spec(
            &specs,
            "SPEC-2",
            "gwt-spec: Workspace shell",
            "in-progress",
            &[("spec.md", "# spec2\n")],
        );

        let report = plan(&specs).unwrap();
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].source, "SPEC-1");
        assert_eq!(report.entries[0].title, "gwt-spec: Terminal emulation");
        assert_eq!(report.entries[0].phase_label, "phase/done");
        assert!(report.entries[0]
            .sections
            .contains_key(&SectionName("spec".to_string())));
        assert!(report.entries[0]
            .sections
            .contains_key(&SectionName("tasks".to_string())));
        assert_eq!(report.entries[1].source, "SPEC-2");
        assert_eq!(report.entries[1].phase_label, "phase/implementation");
    }

    #[test]
    fn plan_sorts_specs_numerically() {
        let tmp = TempDir::new().unwrap();
        let specs = tmp.path().join("specs");
        fs::create_dir_all(&specs).unwrap();
        for n in [10u32, 2, 1, 3] {
            write_spec(
                &specs,
                &format!("SPEC-{n}"),
                &format!("gwt-spec: S{n}"),
                "draft",
                &[("spec.md", "x")],
            );
        }
        let report = plan(&specs).unwrap();
        let order: Vec<String> = report.entries.iter().map(|e| e.source.clone()).collect();
        assert_eq!(order, vec!["SPEC-1", "SPEC-2", "SPEC-3", "SPEC-10"]);
    }

    #[test]
    fn plan_skips_directories_without_metadata() {
        let tmp = TempDir::new().unwrap();
        let specs = tmp.path().join("specs");
        fs::create_dir_all(specs.join("SPEC-9")).unwrap();
        fs::write(specs.join("SPEC-9").join("spec.md"), "x").unwrap();
        write_spec(
            &specs,
            "SPEC-1",
            "gwt-spec: Proper",
            "draft",
            &[("spec.md", "x")],
        );
        let report = plan(&specs).unwrap();
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, "SPEC-9");
    }

    #[test]
    fn plan_collects_contracts_and_checklists() {
        let tmp = TempDir::new().unwrap();
        let specs = tmp.path().join("specs");
        fs::create_dir_all(&specs).unwrap();
        let dir = write_spec(
            &specs,
            "SPEC-1",
            "gwt-spec: S",
            "draft",
            &[("spec.md", "x")],
        );
        let contracts = dir.join("contracts");
        fs::create_dir_all(&contracts).unwrap();
        fs::write(contracts.join("api.yaml"), "openapi: 3.1").unwrap();
        let checklists = dir.join("checklists");
        fs::create_dir_all(&checklists).unwrap();
        fs::write(checklists.join("tdd.md"), "- [ ] test").unwrap();

        let report = plan(&specs).unwrap();
        let sections = &report.entries[0].sections;
        assert!(sections.contains_key(&SectionName("contract/api.yaml".to_string())));
        assert!(sections.contains_key(&SectionName("checklist/tdd.md".to_string())));
    }

    #[test]
    fn execute_runs_create_spec_for_each_entry() {
        use crate::client::fake::FakeIssueClient;

        let tmp = TempDir::new().unwrap();
        let specs = tmp.path().join("specs");
        fs::create_dir_all(&specs).unwrap();
        write_spec(
            &specs,
            "SPEC-1",
            "gwt-spec: One",
            "draft",
            &[("spec.md", "s1")],
        );
        write_spec(
            &specs,
            "SPEC-2",
            "gwt-spec: Two",
            "in-progress",
            &[("spec.md", "s2")],
        );
        let report = plan(&specs).unwrap();

        let cache_dir = tmp.path().join("cache");
        let cache = Cache::new(cache_dir);
        let client = FakeIssueClient::new();
        let results = execute(&report, client, cache);
        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(r.is_ok(), "migration entry failed: {r:?}");
        }
        let first = results[0].as_ref().unwrap();
        assert_eq!(first.source, "SPEC-1");
        assert_eq!(first.phase_label, "phase/draft");
        assert!(first.new_issue_number >= 1);
    }
}
