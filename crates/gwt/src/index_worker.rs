use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use gwt_core::{
    index::{
        paths::gwt_index_root,
        runtime::{reconcile_repo, PythonRunnerSpawner, ReconcileOptions, RunnerSpawner},
    },
    index_coordinator::{IndexCoordinator, JobAdmission, JobOutcome, JobPriority, TargetKey},
    repo_hash::RepoHash,
    worktree_hash::compute_worktree_hash,
};
use serde::Serialize;

/// Determine `RepoHash` for the given repository root by shelling out to
/// `git remote get-url origin`. Returns `None` if no origin is configured.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    gwt_core::repo_hash::detect_repo_hash(repo_root).or_else(|| {
        let index_root = resolve_project_index_repo_root(repo_root)?;
        gwt_core::repo_hash::detect_repo_hash(&index_root)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIndexGitContext {
    pub repo_root: PathBuf,
    pub current_worktree_root: Option<PathBuf>,
}

pub fn project_index_git_context(project_root: &Path) -> Option<ProjectIndexGitContext> {
    if !project_root.exists() {
        return None;
    }
    let current_worktree_root = resolve_current_git_worktree_root(project_root);
    let repo_root = gwt_git::worktree::main_worktree_root(project_root)
        .ok()
        .map(canonicalize_path)
        .or_else(|| current_worktree_root.clone())?;
    Some(ProjectIndexGitContext {
        repo_root,
        current_worktree_root,
    })
}

pub fn resolve_project_index_repo_root(project_root: &Path) -> Option<PathBuf> {
    project_index_git_context(project_root).map(|context| context.repo_root)
}

pub fn default_project_index_worktree_root(project_root: &Path) -> Option<PathBuf> {
    let context = project_index_git_context(project_root)?;
    if context.current_worktree_root.is_some() {
        return context.current_worktree_root;
    }
    process_cwd_worktree_root_for_repo(&context.repo_root)
        .or_else(|| first_active_worktree_path(&context.repo_root))
        .or(Some(context.repo_root))
}

pub fn bootstrap_project_index_for_path(project_root: &Path) -> Result<(), String> {
    if test_fixture_status_path().is_some() {
        tracing::info!(
            target: "gwt::index",
            project_root = %project_root.display(),
            "GWT_INDEX_TEST_FIXTURE applied: skipping project index bootstrap"
        );
        return Ok(());
    }
    let runtime_started = Instant::now();
    gwt_core::runtime::ensure_project_index_runtime().map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %project_root.display(),
        elapsed_ms = runtime_started.elapsed().as_millis() as u64,
        "project index runtime ensured for bootstrap"
    );
    let spawner = PythonRunnerSpawner {
        python_executable: project_index_python_path(),
        runner_script: gwt_core::runtime::project_index_runner_path(),
    };
    bootstrap_project_index_for_path_with(project_root, &gwt_index_root(), &spawner)
}

/// Per-cell index rebuild scope used by the orchestrator, in-flight set, and
/// per-cell IPC. SPEC-1939 US-5#6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexRebuildScope {
    Issues,
    Specs,
    Memory,
    Discussions,
    Board,
    Works,
    Files,
    #[serde(rename = "files-docs")]
    FilesDocs,
}

impl IndexRebuildScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Issues => "issues",
            Self::Specs => "specs",
            Self::Memory => "memory",
            Self::Discussions => "discussions",
            Self::Board => "board",
            Self::Works => "works",
            Self::Files => "files",
            Self::FilesDocs => "files-docs",
        }
    }

    pub fn requires_worktree_hash(self) -> bool {
        matches!(self, Self::Files | Self::FilesDocs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectIndexStatusState {
    Ready,
    Skipped,
    Error,
    RepairRequired,
    Repairing,
}

impl ProjectIndexStatusState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Skipped => "skipped",
            Self::Error => "error",
            Self::RepairRequired => "repair_required",
            Self::Repairing => "repairing",
        }
    }
}

impl fmt::Display for ProjectIndexStatusState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct RebuildProgress {
    pub scopes_done: u32,
    pub scopes_total: u32,
}

/// Per-scope health detail for `(scope, worktree?)` pairs surfaced via
/// `ProjectIndexStatus` events (SPEC-1939 FR-038 / FR-053).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ScopeHealthView {
    pub healthy: bool,
    pub repair_required: bool,
    pub document_count: u64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_residue_detected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_repair_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl ScopeHealthView {
    pub fn ready(document_count: u64) -> Self {
        Self {
            healthy: true,
            repair_required: false,
            document_count,
            reason: "ready".to_string(),
            legacy_residue_detected: None,
            last_repair_at: None,
        }
    }

    pub fn unhealthy(reason: impl Into<String>) -> Self {
        Self {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: reason.into(),
            legacy_residue_detected: None,
            last_repair_at: None,
        }
    }
}

/// Aggregated scope health: `issues` and `specs` are repo-shared and emitted
/// once per project, while `files` and `files_docs` are per-worktree, keyed
/// by `worktree_hash` (SPEC-1939 FR-053).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ProjectIndexScopes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues: Option<ScopeHealthView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specs: Option<ScopeHealthView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<ScopeHealthView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discussions: Option<ScopeHealthView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<ScopeHealthView>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub files: BTreeMap<String, ScopeHealthView>,
    #[serde(
        rename = "files-docs",
        skip_serializing_if = "BTreeMap::is_empty",
        default
    )]
    pub files_docs: BTreeMap<String, ScopeHealthView>,
}

impl ProjectIndexScopes {
    pub fn is_empty(&self) -> bool {
        self.issues.is_none()
            && self.specs.is_none()
            && self.memory.is_none()
            && self.discussions.is_none()
            && self.board.is_none()
            && self.files.is_empty()
            && self.files_docs.is_empty()
    }
}

/// Worktree metadata indexed by worktree hash for the per-worktree health
/// table (SPEC-1939 FR-053).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct WorktreeMeta {
    pub branch: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectIndexStatusCoverageScope {
    CurrentWorktree,
    AllWorktrees,
    PartialAllWorktrees,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ProjectIndexStatusCoverage {
    pub scope: ProjectIndexStatusCoverageScope,
    pub probed_worktrees: usize,
    pub total_worktrees: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ProjectIndexStatusView {
    pub state: ProjectIndexStatusState,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<RebuildProgress>,
    #[serde(skip_serializing_if = "ProjectIndexScopes::is_empty", default)]
    pub scopes: ProjectIndexScopes,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub worktrees: BTreeMap<String, WorktreeMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<ProjectIndexStatusCoverage>,
}

impl ProjectIndexStatusView {
    pub fn new(state: ProjectIndexStatusState, detail: impl Into<String>) -> Self {
        Self {
            state,
            detail: detail.into(),
            repair_started_at: None,
            progress: None,
            scopes: ProjectIndexScopes::default(),
            worktrees: BTreeMap::new(),
            coverage: None,
        }
    }

    fn with_coverage(mut self, coverage: ProjectIndexStatusCoverage) -> Self {
        self.coverage = Some(coverage);
        self
    }
}

pub fn project_index_status_for_path(project_root: &Path) -> ProjectIndexStatusView {
    if let Some(fixture) = load_test_fixture_status() {
        return fixture;
    }
    match project_index_status_for_path_inner(project_root) {
        Ok(status) => status,
        Err(error) => ProjectIndexStatusView::new(ProjectIndexStatusState::Error, error),
    }
}

/// Per-worktree probe input used by the aggregator. Each entry corresponds to
/// one row in `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeProbeInput {
    pub worktree_hash: String,
    pub branch: String,
    pub path: PathBuf,
}

/// Per-worktree probe outcome consumed by the aggregator. The `status_payload`
/// is the parsed runner output for that worktree, or an error string if the
/// runner failed for the specific worktree.
#[derive(Debug, Clone)]
pub struct WorktreeProbeOutcome {
    pub input: WorktreeProbeInput,
    pub status_payload: Result<serde_json::Value, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitWorktreeListEntry {
    path: PathBuf,
    branch: Option<String>,
    bare: bool,
    prunable: bool,
}

/// Translate a single scope sub-payload from the runner status JSON into a
/// [`ScopeHealthView`]. Returns `None` if the payload is not an object.
pub fn parse_scope_health(payload: &serde_json::Value) -> Option<ScopeHealthView> {
    let obj = payload.as_object()?;
    let healthy = obj
        .get("healthy")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let repair_required = obj
        .get("repair_required")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(!healthy);
    let document_count = obj
        .get("document_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let reason = obj
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let legacy_residue_detected = obj
        .get("legacy_residue_detected")
        .and_then(serde_json::Value::as_bool);
    let last_repair_at = obj
        .get("last_repair_at")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    Some(ScopeHealthView {
        healthy,
        repair_required,
        document_count,
        reason,
        legacy_residue_detected,
        last_repair_at,
    })
}

/// Build the aggregated [`ProjectIndexStatusView`] from per-worktree probes.
///
/// `issues` and `specs` are repo-shared and are taken from the first probe
/// that supplies them. `files` and `files-docs` are per-worktree and indexed
/// by `worktree_hash`. The summary `state` is `Error` when any probe failed,
/// `RepairRequired` when any scope is unhealthy, and `Ready` otherwise.
pub fn build_aggregated_status_view(
    runner_asset_hash: &str,
    probes: &[WorktreeProbeOutcome],
) -> ProjectIndexStatusView {
    let mut scopes = ProjectIndexScopes::default();
    let mut worktrees: BTreeMap<String, WorktreeMeta> = BTreeMap::new();
    let mut probe_error: Option<String> = None;

    for probe in probes {
        worktrees.insert(
            probe.input.worktree_hash.clone(),
            WorktreeMeta {
                branch: probe.input.branch.clone(),
                path: probe.input.path.display().to_string(),
            },
        );

        let payload = match &probe.status_payload {
            Ok(value) => value,
            Err(error) => {
                if probe_error.is_none() {
                    probe_error = Some(error.clone());
                }
                continue;
            }
        };

        let Some(status_obj) = payload.get("status").and_then(serde_json::Value::as_object) else {
            continue;
        };

        if scopes.issues.is_none() {
            if let Some(view) = status_obj.get("issues").and_then(parse_scope_health) {
                scopes.issues = Some(view);
            }
        }
        if scopes.specs.is_none() {
            if let Some(view) = status_obj.get("specs").and_then(parse_scope_health) {
                scopes.specs = Some(view);
            }
        }
        if scopes.memory.is_none() {
            if let Some(view) = status_obj.get("memory").and_then(parse_scope_health) {
                scopes.memory = Some(view);
            }
        }
        if scopes.discussions.is_none() {
            if let Some(view) = status_obj.get("discussions").and_then(parse_scope_health) {
                scopes.discussions = Some(view);
            }
        }
        if scopes.board.is_none() {
            if let Some(view) = status_obj.get("board").and_then(parse_scope_health) {
                scopes.board = Some(view);
            }
        }
        if let Some(view) = status_obj.get("files").and_then(parse_scope_health) {
            scopes.files.insert(probe.input.worktree_hash.clone(), view);
        }
        if let Some(view) = status_obj.get("files-docs").and_then(parse_scope_health) {
            scopes
                .files_docs
                .insert(probe.input.worktree_hash.clone(), view);
        }
    }

    let unhealthy_count = count_unhealthy_scopes(&scopes);
    let state = if probe_error.is_some() {
        ProjectIndexStatusState::Error
    } else if unhealthy_count > 0 {
        ProjectIndexStatusState::RepairRequired
    } else {
        ProjectIndexStatusState::Ready
    };

    let detail = match (&state, &probe_error) {
        (ProjectIndexStatusState::Error, Some(reason)) => format!("Status probe failed: {reason}"),
        (ProjectIndexStatusState::RepairRequired, _) => {
            format!("{unhealthy_count} index scope(s) require repair")
        }
        (ProjectIndexStatusState::Ready, _) => {
            format!("Runtime ready; asset {runner_asset_hash}")
        }
        _ => String::new(),
    };

    ProjectIndexStatusView {
        state,
        detail,
        repair_started_at: None,
        progress: None,
        scopes,
        worktrees,
        coverage: None,
    }
}

fn count_unhealthy_scopes(scopes: &ProjectIndexScopes) -> usize {
    let mut count = 0;
    if matches!(&scopes.issues, Some(view) if !view.healthy) {
        count += 1;
    }
    if matches!(&scopes.specs, Some(view) if !view.healthy) {
        count += 1;
    }
    if matches!(&scopes.memory, Some(view) if !view.healthy) {
        count += 1;
    }
    if matches!(&scopes.discussions, Some(view) if !view.healthy) {
        count += 1;
    }
    if matches!(&scopes.board, Some(view) if !view.healthy) {
        count += 1;
    }
    count += scopes.files.values().filter(|view| !view.healthy).count();
    count += scopes
        .files_docs
        .values()
        .filter(|view| !view.healthy)
        .count();
    count
}

/// Environment variable used by Playwright e2e and integration tests to
/// inject a fake `ProjectIndexStatusView` instead of running the real
/// Python runner. The value is a path to a JSON file that deserializes into
/// a `ProjectIndexStatusView`. SPEC-1939 T-IDX-109/110 follow-up scaffolding.
pub const GWT_INDEX_TEST_FIXTURE_ENV: &str = "GWT_INDEX_TEST_FIXTURE";

/// Try to load an aggregated status fixture from `GWT_INDEX_TEST_FIXTURE`.
/// Returns `None` when the env var is absent. Errors during read / parse
/// surface as a synthetic `Error` status so the GUI / orchestrator path
/// behaves the same as a real runner failure.
fn test_fixture_status_path() -> Option<String> {
    let path = std::env::var(GWT_INDEX_TEST_FIXTURE_ENV).ok()?;
    if path.trim().is_empty() {
        return None;
    }
    Some(path)
}

fn load_test_fixture_status() -> Option<ProjectIndexStatusView> {
    let path = test_fixture_status_path()?;
    match std::fs::read_to_string(&path) {
        Ok(payload) => match serde_json::from_str::<ProjectIndexStatusView>(&payload) {
            Ok(view) => {
                tracing::info!(
                    target: "gwt::index",
                    fixture = %path,
                    state = %view.state,
                    "GWT_INDEX_TEST_FIXTURE applied — bypassing real runner"
                );
                Some(view)
            }
            Err(error) => Some(ProjectIndexStatusView::new(
                ProjectIndexStatusState::Error,
                format!("GWT_INDEX_TEST_FIXTURE parse error: {error}"),
            )),
        },
        Err(error) => Some(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Error,
            format!("GWT_INDEX_TEST_FIXTURE read error ({path}): {error}"),
        )),
    }
}

/// Aggregate per-worktree status for a project root by listing every active
/// worktree from `git worktree list --porcelain`, probing each via the
/// runner, and combining the results with [`build_aggregated_status_view`].
///
/// When `GWT_INDEX_TEST_FIXTURE` is set, the fixture JSON is loaded instead
/// — this is the seam used by Playwright e2e (SPEC-1939 T-IDX-109/110).
pub fn aggregate_project_index_status_for_path(project_root: &Path) -> ProjectIndexStatusView {
    if let Some(fixture) = load_test_fixture_status() {
        return fixture;
    }
    match aggregate_project_index_status_for_path_inner(
        project_root,
        StatusProbeScope::AllWorktrees,
    ) {
        Ok(status) => status,
        Err(error) => ProjectIndexStatusView::new(ProjectIndexStatusState::Error, error),
    }
}

/// Probe only the current worktree. This is the startup path: it gives the
/// project tab and auto-repair orchestrator enough health data without
/// spawning one Python status process for every inactive worktree.
pub fn aggregate_current_worktree_index_status_for_path(
    project_root: &Path,
) -> ProjectIndexStatusView {
    if let Some(fixture) = load_test_fixture_status() {
        return fixture;
    }
    match aggregate_project_index_status_for_path_inner(
        project_root,
        StatusProbeScope::CurrentWorktree,
    ) {
        Ok(status) => status,
        Err(error) => ProjectIndexStatusView::new(ProjectIndexStatusState::Error, error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusProbeScope {
    AllWorktrees,
    CurrentWorktree,
}

#[derive(Debug, Clone)]
struct ProbeInputSelection {
    inputs: Vec<WorktreeProbeInput>,
    coverage: ProjectIndexStatusCoverage,
}

const DEFAULT_ALL_WORKTREE_STATUS_BATCH_LIMIT: usize = 32;

fn all_worktree_status_batch_limit() -> usize {
    std::env::var("GWT_INDEX_STATUS_WORKTREE_BATCH_LIMIT")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(DEFAULT_ALL_WORKTREE_STATUS_BATCH_LIMIT)
}

fn select_probe_inputs_for_scope(
    inputs: Vec<WorktreeProbeInput>,
    probe_scope: StatusProbeScope,
    current_worktree_root: Option<&Path>,
    all_worktree_limit: usize,
) -> ProbeInputSelection {
    let total_worktrees = inputs.len();
    match probe_scope {
        StatusProbeScope::CurrentWorktree => {
            let selected = current_worktree_root
                .map(|root| collect_current_worktree_probe_inputs(inputs, root))
                .unwrap_or_default();
            ProbeInputSelection {
                coverage: ProjectIndexStatusCoverage {
                    scope: ProjectIndexStatusCoverageScope::CurrentWorktree,
                    probed_worktrees: selected.len(),
                    total_worktrees,
                    truncated: false,
                },
                inputs: selected,
            }
        }
        StatusProbeScope::AllWorktrees => {
            let limit = all_worktree_limit.max(1);
            let truncated = total_worktrees > limit;
            let selected: Vec<WorktreeProbeInput> = inputs.into_iter().take(limit).collect();
            ProbeInputSelection {
                coverage: ProjectIndexStatusCoverage {
                    scope: if truncated {
                        ProjectIndexStatusCoverageScope::PartialAllWorktrees
                    } else {
                        ProjectIndexStatusCoverageScope::AllWorktrees
                    },
                    probed_worktrees: selected.len(),
                    total_worktrees,
                    truncated,
                },
                inputs: selected,
            }
        }
    }
}

fn aggregate_project_index_status_for_path_inner(
    project_root: &Path,
    probe_scope: StatusProbeScope,
) -> Result<ProjectIndexStatusView, String> {
    let Some(context) = project_index_git_context(project_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No git worktree detected",
        ));
    };
    let repo_root = context.repo_root;
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No origin remote configured",
        ));
    };
    let selection = select_probe_inputs_for_scope(
        list_worktree_probe_inputs(&repo_root)?,
        probe_scope,
        context.current_worktree_root.as_deref(),
        all_worktree_status_batch_limit(),
    );
    if selection.inputs.is_empty() {
        let detail = match probe_scope {
            StatusProbeScope::CurrentWorktree => {
                "No matching current worktree for project index status"
            }
            StatusProbeScope::AllWorktrees => "No active worktrees for project index status",
        };
        return Ok(
            ProjectIndexStatusView::new(ProjectIndexStatusState::Skipped, detail)
                .with_coverage(selection.coverage),
        );
    }

    let runtime_started = Instant::now();
    let report =
        gwt_core::runtime::ensure_project_index_runtime().map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %project_root.display(),
        elapsed_ms = runtime_started.elapsed().as_millis() as u64,
        "project index runtime ensured for aggregated status"
    );

    let coverage = selection.coverage;
    // Phase 70 FR-393 / AS-13: one batch status process covers every
    // selected worktree instead of one serial Python spawn per worktree.
    let worktree_hashes: Vec<String> = selection
        .inputs
        .iter()
        .map(|input| input.worktree_hash.clone())
        .collect();
    let batch = probe_worktrees_status_batch(&repo_root, &repo_hash, &worktree_hashes);
    let mut probes: Vec<WorktreeProbeOutcome> = Vec::with_capacity(selection.inputs.len());
    match batch {
        Ok(payload) => {
            let runtime = payload
                .get("runtime")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let repo_status = payload
                .get("status")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let worktrees = payload
                .get("worktrees")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            for input in selection.inputs {
                let mut status = repo_status.clone();
                if let (Some(status_obj), Some(worktree_obj)) = (
                    status.as_object_mut(),
                    worktrees
                        .get(&input.worktree_hash)
                        .and_then(serde_json::Value::as_object),
                ) {
                    for (key, value) in worktree_obj {
                        status_obj.insert(key.clone(), value.clone());
                    }
                }
                probes.push(WorktreeProbeOutcome {
                    input,
                    status_payload: Ok(serde_json::json!({
                        "ok": true,
                        "runtime": runtime.clone(),
                        "status": status,
                    })),
                });
            }
        }
        Err(error) => {
            for input in selection.inputs {
                probes.push(WorktreeProbeOutcome {
                    input,
                    status_payload: Err(error.clone()),
                });
            }
        }
    }

    let mut view = build_aggregated_status_view(report.runner_hash.as_str(), &probes);
    if coverage.truncated {
        view.detail = format!(
            "Partial status: probed {}/{} worktrees; {}",
            coverage.probed_worktrees, coverage.total_worktrees, view.detail
        );
    }
    view.coverage = Some(coverage);
    Ok(view)
}

fn collect_current_worktree_probe_inputs(
    inputs: Vec<WorktreeProbeInput>,
    current_worktree_root: &Path,
) -> Vec<WorktreeProbeInput> {
    let current = canonicalize_path(current_worktree_root.to_path_buf());
    inputs
        .into_iter()
        .filter(|input| canonicalize_path(input.path.clone()) == current)
        .collect()
}

/// Enumerate worktrees under `repo_root` via `git worktree list --porcelain`.
/// Returned entries pair each worktree's canonicalized path with its branch
/// label and pre-computed `worktree_hash`.
pub fn list_worktree_probe_inputs(repo_root: &Path) -> Result<Vec<WorktreeProbeInput>, String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    parse_worktree_probe_inputs(&String::from_utf8_lossy(&output.stdout))
}

fn parse_worktree_probe_inputs(stdout: &str) -> Result<Vec<WorktreeProbeInput>, String> {
    let mut inputs = Vec::new();
    for entry in parse_git_worktree_porcelain(stdout) {
        if !git_worktree_entry_is_active(&entry) {
            continue;
        }
        push_worktree_probe_input(&mut inputs, entry.path, entry.branch)?;
    }

    Ok(inputs)
}

fn parse_git_worktree_paths(stdout: &str) -> Vec<PathBuf> {
    parse_git_worktree_porcelain(stdout)
        .into_iter()
        .filter(git_worktree_entry_is_active)
        .map(|entry| entry.path)
        .collect()
}

fn git_worktree_entry_is_active(entry: &GitWorktreeListEntry) -> bool {
    !entry.bare && !entry.prunable && entry.path.exists()
}

fn parse_git_worktree_porcelain(stdout: &str) -> Vec<GitWorktreeListEntry> {
    let mut entries = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut current_bare = false;
    let mut current_prunable = false;

    let flush = |entries: &mut Vec<GitWorktreeListEntry>,
                 current_path: &mut Option<PathBuf>,
                 current_branch: &mut Option<String>,
                 current_bare: &mut bool,
                 current_prunable: &mut bool| {
        if let Some(path) = current_path.take() {
            entries.push(GitWorktreeListEntry {
                path,
                branch: current_branch.take(),
                bare: *current_bare,
                prunable: *current_prunable,
            });
        }
        *current_bare = false;
        *current_prunable = false;
    };

    for line in stdout.lines().chain(std::iter::once("")) {
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(
                &mut entries,
                &mut current_path,
                &mut current_branch,
                &mut current_bare,
                &mut current_prunable,
            );
            current_path = Some(canonicalize_path(PathBuf::from(rest)));
        } else if let Some(branch) = line.strip_prefix("branch ") {
            current_branch = Some(branch.trim_start_matches("refs/heads/").to_string());
        } else if line == "bare" {
            current_bare = true;
        } else if line.starts_with("prunable") {
            current_prunable = true;
        } else if line.is_empty() {
            flush(
                &mut entries,
                &mut current_path,
                &mut current_branch,
                &mut current_bare,
                &mut current_prunable,
            );
        }
    }

    entries
}

fn push_worktree_probe_input(
    inputs: &mut Vec<WorktreeProbeInput>,
    path: PathBuf,
    branch: Option<String>,
) -> Result<(), String> {
    let worktree_hash =
        compute_worktree_hash(&path).map_err(|err| format!("compute worktree hash: {err}"))?;
    inputs.push(WorktreeProbeInput {
        worktree_hash: worktree_hash.to_string(),
        branch: branch.unwrap_or_else(|| "(detached)".to_string()),
        path,
    });
    Ok(())
}

/// One batch status process for every selected worktree (FR-393 / AS-13).
fn probe_worktrees_status_batch(
    repo_root: &Path,
    repo_hash: &RepoHash,
    worktree_hashes: &[String],
) -> Result<serde_json::Value, String> {
    let runner_started = Instant::now();
    let output = gwt_core::process::hidden_command(project_index_python_path())
        .arg(gwt_core::runtime::project_index_runner_path())
        .arg("--action")
        .arg("status")
        .arg("--repo-hash")
        .arg(repo_hash.as_str())
        .arg("--worktree-hashes")
        .arg(worktree_hashes.join(","))
        .current_dir(repo_root)
        .output()
        .map_err(|err| format!("run project index status: {err}"))?;
    if !output.status.success() {
        // FR-393: a failed runner invocation invalidates the positive probe
        // cache so the next ensure re-verifies the runtime.
        gwt_core::runtime::invalidate_project_index_probe_cache();
        tracing::warn!(
            target: "gwt::index",
            project_root = %repo_root.display(),
            worktree_count = worktree_hashes.len(),
            elapsed_ms = runner_started.elapsed().as_millis() as u64,
            exit_status = %output.status,
            "project index batch status runner failed"
        );
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!("runner exit {}: {detail}", output.status));
    }
    tracing::debug!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        worktree_count = worktree_hashes.len(),
        elapsed_ms = runner_started.elapsed().as_millis() as u64,
        "project index batch status runner completed"
    );
    serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("parse project index status: {err}"))
}

/// TTL cache for the aggregated status. Used to debounce repeated callers
/// (bootstrap, frontend reload, Settings.Index open) within a short window.
pub struct AggregatedStatusCache {
    entries: Mutex<HashMap<PathBuf, (Instant, ProjectIndexStatusView)>>,
    ttl: Duration,
}

impl AggregatedStatusCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    pub fn get_or_compute<F>(&self, project_root: &Path, compute: F) -> ProjectIndexStatusView
    where
        F: FnOnce(&Path) -> ProjectIndexStatusView,
    {
        let key = canonicalize_path(project_root.to_path_buf());
        {
            let entries = self.entries.lock().expect("aggregator cache");
            if let Some((when, value)) = entries.get(&key) {
                if when.elapsed() < self.ttl {
                    return value.clone();
                }
            }
        }
        let value = compute(&key);
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(key, (Instant::now(), value.clone()));
        }
        value
    }

    pub fn invalidate(&self, project_root: &Path) {
        let key = canonicalize_path(project_root.to_path_buf());
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&key);
        }
    }
}

const AGGREGATOR_DEFAULT_TTL: Duration = Duration::from_secs(2);

pub fn global_aggregated_status_cache() -> &'static AggregatedStatusCache {
    static CACHE: OnceLock<AggregatedStatusCache> = OnceLock::new();
    CACHE.get_or_init(|| AggregatedStatusCache::new(AGGREGATOR_DEFAULT_TTL))
}

/// Per-cell rebuild target identified by `(scope, worktree_hash?)`. `Issues`
/// and `Specs` are repo-shared and carry `worktree_hash = None`.
pub type RebuildTarget = (IndexRebuildScope, Option<String>);

/// Spawner used by the auto-rebuild orchestrator to actually run a rebuild
/// for `(scope, worktree_hash?)`. Production wires this to the same path the
/// CLI `gwt index rebuild` uses; tests inject a fake.
pub trait IndexRebuildSpawner: Send + 'static {
    fn rebuild(
        &self,
        project_root: &Path,
        scope: IndexRebuildScope,
        worktree_hash: Option<&str>,
    ) -> Result<(), String>;
}

/// Type alias for rebuild runner closures that talk to the actual Python
/// runtime. The default runner ([`default_rebuild_runner`]) shells out to
/// `chroma_index_runner.py` via the existing CLI helpers; tests inject a
/// fake.
pub type IndexRebuildRunnerFn =
    dyn Fn(&Path, IndexRebuildScope, Option<&str>) -> Result<(), String> + Send + Sync;

/// How long a caller waits for job admission metadata operations.
const INDEX_JOB_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a job owner waits for the host-wide heavy lease. Rebuilds queue
/// behind whichever embedding build currently owns the model slot (FR-379),
/// so this must cover a full large-repo build.
const INDEX_HEAVY_LEASE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a joined caller waits for the shared job outcome.
const INDEX_SHARED_JOB_WAIT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Result of a coordinated index job (SPEC #1939 Phase 70 FR-382).
#[derive(Debug)]
pub(crate) enum CoordinatedRun<T> {
    /// This process owned the job and ran `build`.
    Ran(T),
    /// An equivalent job for the same target completed while we waited; the
    /// duplicate build was skipped.
    Coalesced,
}

/// One round of a coordinated build (FR-389).
pub(crate) enum BuildStep<T> {
    Done(T),
    /// The runner parked a resumable continuation to hand the heavy lease to
    /// a higher-priority claimant; the owner must re-acquire and resume.
    Yielded,
}

/// Backstop against a pathological yield loop; each round is additionally
/// bounded by the heavy lease timeout.
const MAX_BUILD_YIELDS: u32 = 1000;

/// Run one index build for `(repo_hash, scope, worktree_hash?)` through the
/// host-wide coordinator: at most one model-loaded runner tree host-wide
/// (FR-379), same-target requests coalesce into one shared job (FR-382),
/// and a yielded background build releases the heavy lease before resuming
/// (FR-389).
pub(crate) fn run_coordinated_index_job<T>(
    repo_hash: &str,
    scope_label: &str,
    worktree_hash: Option<&str>,
    priority: JobPriority,
    mut build: impl FnMut() -> Result<BuildStep<T>, String>,
) -> Result<CoordinatedRun<T>, String> {
    let coordinator = IndexCoordinator::open_default()
        .map_err(|err| format!("index coordinator unavailable: {err}"))?;
    let key = match worktree_hash {
        Some(worktree) => TargetKey::worktree(repo_hash, scope_label, worktree),
        None => TargetKey::repo_shared(repo_hash, scope_label),
    };
    // One retry when the previous owner vanished without publishing.
    for _ in 0..2 {
        let admission = coordinator
            .request_job(&key, priority, INDEX_JOB_ADMISSION_TIMEOUT)
            .map_err(|err| format!("index job admission failed: {err}"))?;
        match admission {
            JobAdmission::Owner(guard) => {
                let mut yields: u32 = 0;
                loop {
                    let heavy = guard
                        .acquire_heavy(INDEX_HEAVY_LEASE_TIMEOUT)
                        .map_err(|err| format!("index heavy lease failed: {err}"))?;
                    let step = build();
                    drop(heavy);
                    match step {
                        Ok(BuildStep::Done(value)) => {
                            guard
                                .complete(JobOutcome::Completed)
                                .map_err(|err| format!("index job completion failed: {err}"))?;
                            return Ok(CoordinatedRun::Ran(value));
                        }
                        Ok(BuildStep::Yielded) => {
                            yields += 1;
                            if yields > MAX_BUILD_YIELDS {
                                let message =
                                    "index build yielded too many times without completing"
                                        .to_string();
                                let _ = guard.complete(JobOutcome::Failed {
                                    message: message.clone(),
                                });
                                return Err(message);
                            }
                            // Give the pending higher-priority claimant a
                            // chance to take the freshly released lease
                            // before re-queueing; acquire_heavy then defers
                            // for as long as higher-priority claimants stay
                            // pending (FR-383).
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(message) => {
                            guard
                                .complete(JobOutcome::Failed {
                                    message: message.clone(),
                                })
                                .map_err(|err| format!("index job completion failed: {err}"))?;
                            return Err(message);
                        }
                    }
                }
            }
            JobAdmission::Joined(waiter) => {
                let outcome = waiter
                    .wait(INDEX_SHARED_JOB_WAIT_TIMEOUT)
                    .map_err(|err| format!("shared index job wait failed: {err}"))?;
                match outcome {
                    JobOutcome::Completed => return Ok(CoordinatedRun::Coalesced),
                    JobOutcome::Failed { message } => return Err(message),
                    JobOutcome::OwnerGone => continue,
                }
            }
        }
    }
    Err("index job owner disappeared repeatedly without publishing an outcome".to_string())
}

/// Production rebuild runner used by the background auto-rebuild
/// orchestrator. Runs at background priority so interactive and manual work
/// preempt the heavy lease (FR-383).
pub fn default_rebuild_runner(
    project_root: &Path,
    scope: IndexRebuildScope,
    worktree_hash: Option<&str>,
) -> Result<(), String> {
    rebuild_index_target(project_root, scope, worktree_hash, JobPriority::Background)
}

/// User-initiated rebuild runner (per-cell GUI rebuild, `gwt index rebuild`).
pub fn manual_rebuild_runner(
    project_root: &Path,
    scope: IndexRebuildScope,
    worktree_hash: Option<&str>,
) -> Result<(), String> {
    rebuild_index_target(
        project_root,
        scope,
        worktree_hash,
        JobPriority::ManualRebuild,
    )
}

/// Resolve an `IndexContext` for the requested `(project_root,
/// worktree_hash?)` and invoke the runner via the same path used by
/// `gwt index rebuild`, gated by the host-wide index coordinator so
/// concurrent CLI/GUI/agent rebuilds coalesce and the heavy runner stays
/// exclusive host-wide.
pub fn rebuild_index_target(
    project_root: &Path,
    scope: IndexRebuildScope,
    worktree_hash: Option<&str>,
    priority: JobPriority,
) -> Result<(), String> {
    use crate::cli::index::runtime::{
        format_runner_failure, resolve_index_context, run_runner_rebuild, RebuildAction,
    };

    let mut ctx = resolve_index_context(project_root).map_err(|err| err.to_string())?;
    if let Some(target_hash) = worktree_hash {
        let inputs = list_worktree_probe_inputs(&ctx.project_root)?;
        let target = inputs
            .into_iter()
            .find(|input| input.worktree_hash == target_hash)
            .ok_or_else(|| format!("worktree with hash {target_hash} not found"))?;
        ctx.project_root = target.path;
        ctx.worktree_hash = target_hash.to_string();
    }
    let action = match scope {
        IndexRebuildScope::Issues => RebuildAction {
            label: "issues",
            action: "index-issues",
            scope: None,
            needs_worktree_hash: false,
        },
        IndexRebuildScope::Specs => RebuildAction {
            label: "specs",
            action: "index-specs",
            scope: None,
            needs_worktree_hash: false,
        },
        IndexRebuildScope::Memory => RebuildAction {
            label: "memory",
            action: "index-memory",
            scope: None,
            needs_worktree_hash: false,
        },
        IndexRebuildScope::Discussions => RebuildAction {
            label: "discussions",
            action: "index-discussions",
            scope: None,
            needs_worktree_hash: false,
        },
        IndexRebuildScope::Board => RebuildAction {
            label: "board",
            action: "index-board",
            scope: None,
            needs_worktree_hash: false,
        },
        IndexRebuildScope::Works => RebuildAction {
            label: "works",
            action: "index-works",
            scope: None,
            needs_worktree_hash: false,
        },
        IndexRebuildScope::Files => RebuildAction {
            label: "files",
            action: "index-files",
            scope: Some("files"),
            needs_worktree_hash: true,
        },
        IndexRebuildScope::FilesDocs => RebuildAction {
            label: "files-docs",
            action: "index-files",
            scope: Some("files-docs"),
            needs_worktree_hash: true,
        },
    };
    let coordinator_worktree = action
        .needs_worktree_hash
        .then(|| ctx.worktree_hash.clone());
    let qos = match priority {
        JobPriority::Background => "background",
        JobPriority::ManualRebuild | JobPriority::InteractiveSearch => "interactive",
    };
    let run = run_coordinated_index_job(
        ctx.repo_hash.as_str(),
        action.label,
        coordinator_worktree.as_deref(),
        priority,
        || {
            let rebuild_started = Instant::now();
            let rebuild_label = action.label;
            let rebuild_action = action.action;
            let rebuild_worktree_hash = ctx.worktree_hash.clone();
            tracing::info!(
                target: "gwt::index",
                scope = rebuild_label,
                worktree_hash = %rebuild_worktree_hash,
                action = rebuild_action,
                "project index rebuild started"
            );
            let output = run_runner_rebuild(&ctx, action, qos).map_err(|err| err.to_string())?;
            tracing::info!(
                target: "gwt::index",
                scope = rebuild_label,
                worktree_hash = %rebuild_worktree_hash,
                action = rebuild_action,
                elapsed_ms = rebuild_started.elapsed().as_millis() as u64,
                exit_status = %output.status,
                "project index rebuild completed"
            );
            if !output.status.success() {
                return Err(format_runner_failure(&output));
            }
            if runner_payload_yielded(&output.stdout) {
                tracing::info!(
                    target: "gwt::index",
                    scope = rebuild_label,
                    "project index rebuild yielded to a higher-priority claimant"
                );
                return Ok(BuildStep::Yielded);
            }
            Ok(BuildStep::Done(()))
        },
    )?;
    if let CoordinatedRun::Coalesced = run {
        tracing::info!(
            target: "gwt::index",
            scope = action.label,
            "project index rebuild coalesced into a concurrent equivalent job"
        );
    }
    Ok(())
}

/// True when the runner parked a resumable continuation instead of
/// completing (Phase 70 FR-389 `yielded` payload field).
pub(crate) fn runner_payload_yielded(stdout: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(stdout)
        .ok()
        .and_then(|payload| payload.get("yielded").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// Collect every unhealthy `(scope, worktree_hash?)` cell from an aggregated
/// status payload, in a deterministic order: issues, specs, files, files-docs.
pub fn collect_unhealthy_rebuild_targets(scopes: &ProjectIndexScopes) -> Vec<RebuildTarget> {
    let mut targets = Vec::new();
    if matches!(&scopes.issues, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Issues, None));
    }
    if matches!(&scopes.specs, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Specs, None));
    }
    if matches!(&scopes.memory, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Memory, None));
    }
    if matches!(&scopes.discussions, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Discussions, None));
    }
    if matches!(&scopes.board, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Board, None));
    }
    for (wt_hash, view) in &scopes.files {
        if !view.healthy {
            targets.push((IndexRebuildScope::Files, Some(wt_hash.clone())));
        }
    }
    for (wt_hash, view) in &scopes.files_docs {
        if !view.healthy {
            targets.push((IndexRebuildScope::FilesDocs, Some(wt_hash.clone())));
        }
    }
    targets
}

/// Collect startup auto-repair targets for the currently opened worktree.
///
/// Repo-shared scopes (`issues`, `specs`) are still eligible, but per-worktree
/// file scopes are limited to `project_root`. Inactive worktrees remain
/// visible in the aggregated health table and can be repaired explicitly from
/// Settings.Index.
pub fn collect_unhealthy_rebuild_targets_for_project_root(
    scopes: &ProjectIndexScopes,
    project_root: &Path,
) -> Vec<RebuildTarget> {
    let current_hash = compute_worktree_hash(project_root).ok();
    collect_unhealthy_rebuild_targets_for_worktree_hash(
        scopes,
        current_hash.as_ref().map(|hash| hash.as_str()),
    )
}

fn collect_unhealthy_rebuild_targets_for_worktree_hash(
    scopes: &ProjectIndexScopes,
    current_worktree_hash: Option<&str>,
) -> Vec<RebuildTarget> {
    let mut targets = Vec::new();
    if matches!(&scopes.issues, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Issues, None));
    }
    if matches!(&scopes.specs, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Specs, None));
    }
    if matches!(&scopes.memory, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Memory, None));
    }
    if matches!(&scopes.discussions, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Discussions, None));
    }
    if matches!(&scopes.board, Some(view) if !view.healthy) {
        targets.push((IndexRebuildScope::Board, None));
    }
    if let Some(current_hash) = current_worktree_hash {
        if matches!(scopes.files.get(current_hash), Some(view) if !view.healthy) {
            targets.push((IndexRebuildScope::Files, Some(current_hash.to_string())));
        }
        if matches!(scopes.files_docs.get(current_hash), Some(view) if !view.healthy) {
            targets.push((IndexRebuildScope::FilesDocs, Some(current_hash.to_string())));
        }
    }
    targets
}

/// Auto-repair every unhealthy scope reported in `initial_status` by
/// rebuilding them serially in a background thread.
///
/// Behaviour:
/// - Returns `None` immediately if `initial_status.state` is not
///   `RepairRequired` or no unhealthy scope is detected.
/// - Synchronously emits one `Repairing(0/N)` event so callers see the
///   transition out of `repair_required` before the rebuild work begins.
/// - Spawns a worker thread that rebuilds each unhealthy scope in order via
///   `spawner`, emitting `Repairing(i/N)` after each successful step. On the
///   first failure the orchestrator stops and emits a final `Error` view.
/// - When all rebuilds succeed, the orchestrator emits the view returned by
///   `final_status_provider` (typically a freshly aggregated status with the
///   cache invalidated).
pub fn auto_repair_unhealthy_scopes<S, F, P>(
    project_root: PathBuf,
    initial_status: &ProjectIndexStatusView,
    spawner: S,
    final_status_provider: P,
    event_sink: F,
) -> Option<std::thread::JoinHandle<()>>
where
    S: IndexRebuildSpawner,
    F: Fn(ProjectIndexStatusView) + Send + 'static,
    P: FnOnce(&Path) -> ProjectIndexStatusView + Send + 'static,
{
    if initial_status.state != ProjectIndexStatusState::RepairRequired {
        return None;
    }
    let targets = collect_unhealthy_rebuild_targets(&initial_status.scopes);
    auto_repair_unhealthy_targets(
        project_root,
        initial_status,
        targets,
        spawner,
        final_status_provider,
        event_sink,
    )
}

/// Auto-repair a preselected list of unhealthy targets while preserving the
/// full initial status payload in progress events.
pub fn auto_repair_unhealthy_targets<S, F, P>(
    project_root: PathBuf,
    initial_status: &ProjectIndexStatusView,
    targets: Vec<RebuildTarget>,
    spawner: S,
    final_status_provider: P,
    event_sink: F,
) -> Option<std::thread::JoinHandle<()>>
where
    S: IndexRebuildSpawner,
    F: Fn(ProjectIndexStatusView) + Send + 'static,
    P: FnOnce(&Path) -> ProjectIndexStatusView + Send + 'static,
{
    if initial_status.state != ProjectIndexStatusState::RepairRequired {
        return None;
    }
    if targets.is_empty() {
        return None;
    }
    let total = targets.len() as u32;
    let started_at = chrono::Utc::now();
    let initial_scopes = initial_status.scopes.clone();
    let initial_worktrees = initial_status.worktrees.clone();

    // Synchronous transition: switch the badge from `repair_required` to
    // `repairing(0/N)` before any rebuild work starts so observers don't see
    // a stale repair_required steady state.
    event_sink(ProjectIndexStatusView {
        state: ProjectIndexStatusState::Repairing,
        detail: format!("Rebuilding 0 of {total} scope(s)"),
        repair_started_at: Some(started_at),
        progress: Some(RebuildProgress {
            scopes_done: 0,
            scopes_total: total,
        }),
        scopes: initial_scopes.clone(),
        worktrees: initial_worktrees.clone(),
        coverage: None,
    });

    let project_root_for_thread = project_root.clone();
    std::thread::Builder::new()
        .name("gwt-index-orchestrator".to_string())
        .spawn(move || {
            let mut done = 0u32;
            let mut failure: Option<String> = None;
            for (scope, worktree_hash) in &targets {
                match spawner.rebuild(&project_root_for_thread, *scope, worktree_hash.as_deref()) {
                    Ok(()) => {
                        done += 1;
                        event_sink(ProjectIndexStatusView {
                            state: ProjectIndexStatusState::Repairing,
                            detail: format!("Rebuilding {done} of {total} scope(s)"),
                            repair_started_at: Some(started_at),
                            progress: Some(RebuildProgress {
                                scopes_done: done,
                                scopes_total: total,
                            }),
                            scopes: initial_scopes.clone(),
                            worktrees: initial_worktrees.clone(),
                            coverage: None,
                        });
                    }
                    Err(error) => {
                        failure = Some(format!("Rebuild {} failed: {error}", scope.label()));
                        break;
                    }
                }
            }

            let final_view = match failure {
                Some(error) => ProjectIndexStatusView::new(ProjectIndexStatusState::Error, error),
                None => final_status_provider(&project_root_for_thread),
            };
            event_sink(final_view);
        })
        .ok()
}

fn project_index_status_for_path_inner(
    project_root: &Path,
) -> Result<ProjectIndexStatusView, String> {
    let Some(context) = project_index_git_context(project_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No git worktree detected",
        ));
    };
    let repo_root = context.repo_root;
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No origin remote configured",
        ));
    };
    let worktree_root = context
        .current_worktree_root
        .or_else(|| first_active_worktree_path(&repo_root))
        .unwrap_or_else(|| repo_root.clone());
    let worktree_hash = compute_worktree_hash(&worktree_root)
        .map_err(|err| format!("compute worktree hash: {err}"))?;
    let runtime_started = Instant::now();
    let report =
        gwt_core::runtime::ensure_project_index_runtime().map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %project_root.display(),
        elapsed_ms = runtime_started.elapsed().as_millis() as u64,
        "project index runtime ensured for status"
    );
    let runner_started = Instant::now();
    let output = gwt_core::process::hidden_command(project_index_python_path())
        .arg(gwt_core::runtime::project_index_runner_path())
        .arg("--action")
        .arg("status")
        .arg("--repo-hash")
        .arg(repo_hash.as_str())
        .arg("--worktree-hash")
        .arg(worktree_hash.as_str())
        .current_dir(&repo_root)
        .output()
        .map_err(|err| format!("run project index status: {err}"))?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = runner_started.elapsed().as_millis() as u64,
        exit_status = %output.status,
        "project index status runner completed"
    );
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Error,
            format!("runner exit {}: {detail}", output.status),
        ));
    }
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("parse project index status: {err}"))?;
    let unhealthy = payload
        .get("status")
        .and_then(serde_json::Value::as_object)
        .map(|status| {
            status
                .values()
                .filter(|scope| {
                    !scope
                        .get("healthy")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    if unhealthy == 0 {
        Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Ready,
            format!("Runtime ready; asset {}", report.runner_hash),
        ))
    } else {
        Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::RepairRequired,
            format!("{unhealthy} index scope(s) require repair"),
        ))
    }
}

fn read_issue_index_source_fingerprint(index_root: &Path, repo_hash: &RepoHash) -> Option<String> {
    let meta_path = index_root
        .join(repo_hash.as_str())
        .join("issues")
        .join("meta.json");
    let bytes = std::fs::read(meta_path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("source_cache_fingerprint")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn issue_index_needs_rebuild_for_cache(
    index_root: &Path,
    repo_hash: &RepoHash,
    cache_root: &Path,
) -> Result<bool, String> {
    let Some(source) = crate::issue_cache::issue_cache_source_fingerprint(cache_root)? else {
        return Ok(false);
    };
    if source.document_count == 0 {
        return Ok(false);
    }
    Ok(
        read_issue_index_source_fingerprint(index_root, repo_hash).as_deref()
            != Some(source.fingerprint.as_str()),
    )
}

fn refresh_issue_cache_and_index_for_startup<S: RunnerSpawner + ?Sized>(
    repo_root: &Path,
    refresh_project_root: &Path,
    index_root: &Path,
    repo_hash: &RepoHash,
    spawner: &S,
) -> Result<(), String> {
    let cache_root = crate::issue_cache::issue_cache_root_for_repo_hash(repo_hash);
    match crate::issue_cache::sync_issue_cache_from_remote_if_stale_with_fingerprint(
        refresh_project_root,
        &cache_root,
        crate::issue_cache::ISSUE_CACHE_TTL,
    ) {
        Ok(outcome) => {
            tracing::info!(
                target: "gwt::index",
                project_root = %repo_root.display(),
                cache_refreshed = outcome.refreshed,
                source_changed = outcome.source_changed,
                "project index issue cache refresh checked"
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "gwt::index",
                project_root = %repo_root.display(),
                error = %error,
                "project index issue cache refresh failed; continuing with local cache/index"
            );
        }
    }

    if issue_index_needs_rebuild_for_cache(index_root, repo_hash, &cache_root)? {
        spawner
            .spawn_index_issues(repo_hash.as_str(), refresh_project_root, false)
            .map_err(|err| format!("spawn issue index: {err}"))?;
    }
    Ok(())
}

pub fn bootstrap_project_index_for_path_with<S: RunnerSpawner + ?Sized>(
    project_root: &Path,
    index_root: &Path,
    spawner: &S,
) -> Result<(), String> {
    if test_fixture_status_path().is_some() {
        tracing::info!(
            target: "gwt::index",
            project_root = %project_root.display(),
            "GWT_INDEX_TEST_FIXTURE applied: skipping project index bootstrap helper"
        );
        return Ok(());
    }
    let bootstrap_started = Instant::now();
    let Some(context) = project_index_git_context(project_root) else {
        return Ok(());
    };
    let repo_root = context.repo_root;
    let refresh_project_root =
        default_project_index_worktree_root(project_root).unwrap_or_else(|| repo_root.clone());
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(());
    };

    let worktree_list_started = Instant::now();
    let active_worktrees =
        list_git_worktree_paths(&repo_root).unwrap_or_else(|_| vec![repo_root.clone()]);
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = worktree_list_started.elapsed().as_millis() as u64,
        worktree_count = active_worktrees.len(),
        "project index active worktrees listed"
    );
    let reconcile_started = Instant::now();
    reconcile_repo(&ReconcileOptions {
        index_root: index_root.to_path_buf(),
        repo_hash: repo_hash.clone(),
        active_worktree_paths: active_worktrees.clone(),
        legacy_worktree_dirs: active_worktrees,
    })
    .map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = reconcile_started.elapsed().as_millis() as u64,
        "project index repository reconciled"
    );

    let refresh_started = Instant::now();
    refresh_issue_cache_and_index_for_startup(
        &repo_root,
        &refresh_project_root,
        index_root,
        &repo_hash,
        spawner,
    )?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = refresh_started.elapsed().as_millis() as u64,
        "project index issue source refresh checked"
    );
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = bootstrap_started.elapsed().as_millis() as u64,
        "project index bootstrap helper completed"
    );

    Ok(())
}

fn resolve_current_git_worktree_root(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return None;
    }
    Some(canonicalize_path(PathBuf::from(root)))
}

fn first_active_worktree_path(repo_root: &Path) -> Option<PathBuf> {
    list_worktree_probe_inputs(repo_root)
        .ok()?
        .into_iter()
        .next()
        .map(|input| input.path)
}

fn process_cwd_worktree_root_for_repo(repo_root: &Path) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let cwd_worktree = resolve_current_git_worktree_root(&cwd)?;
    let cwd_repo_root = gwt_git::worktree::main_worktree_root(&cwd_worktree)
        .ok()
        .map(canonicalize_path)?;
    if canonicalize_path(cwd_repo_root) == canonicalize_path(repo_root.to_path_buf()) {
        Some(cwd_worktree)
    } else {
        None
    }
}

fn list_git_worktree_paths(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project_root)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let mut worktrees = parse_git_worktree_paths(&String::from_utf8_lossy(&output.stdout));

    if worktrees.is_empty() {
        worktrees.push(canonicalize_path(project_root.to_path_buf()));
    }
    Ok(worktrees)
}

fn canonicalize_path(path: PathBuf) -> PathBuf {
    dunce::canonicalize(&path).unwrap_or(path)
}

pub(crate) fn project_index_python_path() -> PathBuf {
    gwt_core::runtime::project_index_python_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    static GWT_INDEX_TEST_FIXTURE_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct FixtureEnvGuard {
        previous: Option<String>,
    }

    impl FixtureEnvGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var(GWT_INDEX_TEST_FIXTURE_ENV).ok();
            unsafe {
                std::env::set_var(GWT_INDEX_TEST_FIXTURE_ENV, path);
            }
            Self { previous }
        }
    }

    impl Drop for FixtureEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(value) => std::env::set_var(GWT_INDEX_TEST_FIXTURE_ENV, value),
                    None => std::env::remove_var(GWT_INDEX_TEST_FIXTURE_ENV),
                }
            }
        }
    }

    #[test]
    fn worktree_probe_input_parser_skips_bare_prunable_and_missing_worktrees() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bare = temp.path().join("gwt.git");
        let develop = temp.path().join("develop");
        let work = temp.path().join("work");
        let missing = temp.path().join("stale");
        std::fs::create_dir_all(&bare).expect("bare dir");
        std::fs::create_dir_all(&develop).expect("develop dir");
        std::fs::create_dir_all(&work).expect("work dir");
        let stdout = format!(
            "\
worktree {bare}
bare

worktree {missing}
HEAD 1111111111111111111111111111111111111111
prunable gitdir file points to non-existent location

worktree {develop}
HEAD 2222222222222222222222222222222222222222
branch refs/heads/develop

worktree {work}
HEAD 3333333333333333333333333333333333333333
detached

",
            bare = bare.display(),
            missing = missing.display(),
            develop = develop.display(),
            work = work.display(),
        );

        let inputs = parse_worktree_probe_inputs(&stdout).expect("parse probe inputs");

        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].path, canonicalize_path(develop.clone()));
        assert_eq!(inputs[0].branch, "develop");
        assert_eq!(
            inputs[0].worktree_hash,
            compute_worktree_hash(&canonicalize_path(develop.clone()))
                .expect("develop hash")
                .to_string()
        );
        assert_eq!(inputs[1].path, canonicalize_path(work.clone()));
        assert_eq!(inputs[1].branch, "(detached)");
        assert_eq!(
            parse_git_worktree_paths(&stdout),
            vec![canonicalize_path(develop), canonicalize_path(work)]
        );
    }

    struct PanicRunnerSpawner;

    impl RunnerSpawner for PanicRunnerSpawner {
        fn spawn_index_issues(
            &self,
            _repo_hash: &str,
            _project_root: &Path,
            _respect_ttl: bool,
        ) -> std::io::Result<()> {
            panic!("fixture-backed bootstrap must not spawn the real runner");
        }
    }

    fn write_status_fixture(root: &Path, state: ProjectIndexStatusState, detail: &str) -> PathBuf {
        let fixture_path = root.join("status.json");
        let fixture = ProjectIndexStatusView {
            state,
            detail: detail.to_string(),
            repair_started_at: None,
            progress: None,
            scopes: ProjectIndexScopes {
                specs: Some(ScopeHealthView::unhealthy("count_mismatch")),
                ..Default::default()
            },
            worktrees: BTreeMap::new(),
            coverage: None,
        };
        std::fs::write(
            &fixture_path,
            serde_json::to_string(&fixture).expect("serialize"),
        )
        .expect("write fixture");
        fixture_path
    }

    fn init_git_repo_with_origin(path: &Path) {
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "-q", "-b", "develop"])
            .current_dir(path)
            .output()
            .expect("git init");
        assert!(init.status.success(), "git init failed");
        let remote = gwt_core::process::hidden_command("git")
            .args(["remote", "add", "origin", "https://example.com/gwt-e2e.git"])
            .current_dir(path)
            .output()
            .expect("git remote add origin");
        assert!(remote.status.success(), "git remote add origin failed");
    }

    fn run_git_at(path: &Path, args: &[&str]) {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap_or_else(|err| panic!("git {args:?}: {err}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn make_bare_workspace_with_origin(home: &Path) -> (PathBuf, PathBuf) {
        let bare = home.join("gwt.git");
        let bootstrap = home.join(".bootstrap");
        let develop = home.join("develop");
        std::fs::create_dir_all(home).expect("workspace home");
        run_git_at(home, &["init", "--bare", bare.to_str().unwrap()]);
        run_git_at(
            &bare,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/example/gwt.git",
            ],
        );
        run_git_at(home, &["clone", bare.to_str().unwrap(), ".bootstrap"]);
        run_git_at(&bootstrap, &["config", "user.email", "test@example.com"]);
        run_git_at(&bootstrap, &["config", "user.name", "Test User"]);
        run_git_at(&bootstrap, &["checkout", "-b", "develop"]);
        run_git_at(&bootstrap, &["commit", "--allow-empty", "-m", "init"]);
        run_git_at(&bootstrap, &["push", "origin", "develop"]);
        run_git_at(
            &bare,
            &["worktree", "add", develop.to_str().unwrap(), "develop"],
        );
        std::fs::remove_dir_all(&bootstrap).expect("remove bootstrap");
        (bare, develop)
    }

    struct CurrentDirGuard {
        previous: PathBuf,
    }

    impl CurrentDirGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::current_dir().expect("current dir");
            std::env::set_current_dir(path).expect("set current dir");
            Self { previous }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
        }
    }

    #[test]
    fn detect_repo_hash_reads_origin_from_workspace_home_child_bare_repo() {
        let temp = tempfile::tempdir().expect("tempdir");
        make_bare_workspace_with_origin(temp.path());

        let hash = detect_repo_hash(temp.path()).expect("repo hash from workspace home");

        assert_eq!(
            hash.as_str(),
            gwt_core::repo_hash::compute_repo_hash("https://github.com/example/gwt.git").as_str()
        );
    }

    #[test]
    fn default_project_index_worktree_root_prefers_process_cwd_worktree_for_workspace_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (bare, _develop) = make_bare_workspace_with_origin(temp.path());
        let active = temp.path().join("work").join("active");
        std::fs::create_dir_all(active.parent().expect("active parent")).expect("active parent");
        run_git_at(
            &bare,
            &[
                "worktree",
                "add",
                "-b",
                "work/active",
                active.to_str().unwrap(),
                "develop",
            ],
        );
        let _cwd = CurrentDirGuard::set(&active);

        let root = default_project_index_worktree_root(temp.path()).expect("default worktree root");

        assert_eq!(
            canonicalize_path(root),
            canonicalize_path(active),
            "workspace home should use the already-running worktree before the first listed worktree"
        );
    }

    #[test]
    fn project_index_status_state_serializes_stable_protocol_values() {
        let status = ProjectIndexStatusView::new(
            ProjectIndexStatusState::RepairRequired,
            "1 scope requires repair",
        );

        let payload = serde_json::to_value(status).expect("serialize status");

        assert_eq!(payload["state"], "repair_required");
        assert_eq!(payload["detail"], "1 scope requires repair");
    }

    #[test]
    fn project_index_status_state_serializes_repairing_variant() {
        let status = ProjectIndexStatusView::new(
            ProjectIndexStatusState::Repairing,
            "rebuilding 1/4: issues",
        );

        let payload = serde_json::to_value(&status).expect("serialize status");

        assert_eq!(payload["state"], "repairing");
        assert_eq!(payload["detail"], "rebuilding 1/4: issues");
        assert_eq!(ProjectIndexStatusState::Repairing.as_str(), "repairing");
        assert_eq!(
            format!("{}", ProjectIndexStatusState::Repairing),
            "repairing"
        );
    }

    #[test]
    fn project_index_status_view_omits_repair_progress_when_absent() {
        let view = ProjectIndexStatusView {
            state: ProjectIndexStatusState::Ready,
            detail: "Runtime ready".to_string(),
            repair_started_at: None,
            progress: None,
            scopes: ProjectIndexScopes::default(),
            worktrees: BTreeMap::new(),
            coverage: None,
        };

        let payload = serde_json::to_value(&view).expect("serialize ready view");

        assert_eq!(payload["state"], "ready");
        assert!(
            payload.get("repair_started_at").is_none(),
            "repair_started_at should be omitted when None: {payload:?}"
        );
        assert!(
            payload.get("progress").is_none(),
            "progress should be omitted when None: {payload:?}"
        );
    }

    #[test]
    fn project_index_status_view_emits_repair_progress_when_present() {
        let started = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            chrono::NaiveDateTime::parse_from_str("2026-05-07T01:23:45", "%Y-%m-%dT%H:%M:%S")
                .expect("parse fixed timestamp"),
            chrono::Utc,
        );
        let view = ProjectIndexStatusView {
            state: ProjectIndexStatusState::Repairing,
            detail: "rebuilding 1/4: issues".to_string(),
            repair_started_at: Some(started),
            progress: Some(RebuildProgress {
                scopes_done: 1,
                scopes_total: 4,
            }),
            scopes: ProjectIndexScopes::default(),
            worktrees: BTreeMap::new(),
            coverage: None,
        };

        let payload = serde_json::to_value(&view).expect("serialize repairing view");

        assert_eq!(payload["state"], "repairing");
        assert_eq!(payload["repair_started_at"], "2026-05-07T01:23:45Z");
        assert_eq!(payload["progress"]["scopes_done"], 1);
        assert_eq!(payload["progress"]["scopes_total"], 4);
    }

    #[test]
    fn project_index_status_state_variant_set_is_complete() {
        let variants = [
            ProjectIndexStatusState::Ready,
            ProjectIndexStatusState::Skipped,
            ProjectIndexStatusState::Error,
            ProjectIndexStatusState::RepairRequired,
            ProjectIndexStatusState::Repairing,
        ];
        let serialized: Vec<&'static str> = variants.iter().map(|state| state.as_str()).collect();
        assert_eq!(
            serialized,
            vec!["ready", "skipped", "error", "repair_required", "repairing",]
        );
    }

    #[test]
    fn project_index_status_view_omits_aggregated_fields_when_empty() {
        let view = ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "ready");

        let payload = serde_json::to_value(&view).expect("serialize default view");

        assert_eq!(payload["state"], "ready");
        assert!(
            payload.get("scopes").is_none(),
            "scopes should be omitted when default: {payload:?}"
        );
        assert!(
            payload.get("worktrees").is_none(),
            "worktrees should be omitted when default: {payload:?}"
        );
    }

    #[test]
    fn parse_scope_health_extracts_all_known_fields() {
        let payload = serde_json::json!({
            "healthy": false,
            "repair_required": true,
            "document_count": 42,
            "reason": "manifest_missing",
            "legacy_residue_detected": true,
            "last_repair_at": "2026-04-24T06:15:20Z"
        });

        let view = parse_scope_health(&payload).expect("scope payload parses");

        assert!(!view.healthy);
        assert!(view.repair_required);
        assert_eq!(view.document_count, 42);
        assert_eq!(view.reason, "manifest_missing");
        assert_eq!(view.legacy_residue_detected, Some(true));
        assert_eq!(
            view.last_repair_at.expect("timestamp").to_rfc3339(),
            "2026-04-24T06:15:20+00:00"
        );
    }

    #[test]
    fn build_aggregated_status_view_reports_ready_when_all_scopes_healthy() {
        let probes = vec![WorktreeProbeOutcome {
            input: WorktreeProbeInput {
                worktree_hash: "wtAhash".to_string(),
                branch: "develop".to_string(),
                path: PathBuf::from("/abs/wtA"),
            },
            status_payload: Ok(serde_json::json!({
                "status": {
                    "issues": {"healthy": true, "document_count": 100, "reason": "ready"},
                    "specs": {"healthy": true, "document_count": 50, "reason": "ready"},
                    "memory": {"healthy": true, "document_count": 243, "reason": "ready"},
                    "files": {"healthy": true, "document_count": 310, "reason": "ready"},
                    "files-docs": {"healthy": true, "document_count": 16, "reason": "ready"}
                }
            })),
        }];

        let view = build_aggregated_status_view("asset-hash-12", &probes);

        assert_eq!(view.state, ProjectIndexStatusState::Ready);
        assert!(view.detail.contains("asset-hash-12"));
        assert!(view.scopes.issues.is_some());
        assert!(view.scopes.specs.is_some());
        assert!(
            view.scopes.memory.is_some(),
            "memory scope must be present in aggregated view (SPEC-2805)"
        );
        assert_eq!(
            view.scopes.memory.as_ref().map(|view| view.document_count),
            Some(243),
        );
        assert_eq!(view.scopes.files.len(), 1);
        assert_eq!(view.scopes.files_docs.len(), 1);
        assert_eq!(view.worktrees.len(), 1);
    }

    #[test]
    fn build_aggregated_status_view_counts_memory_in_unhealthy_summary() {
        let probes = vec![WorktreeProbeOutcome {
            input: WorktreeProbeInput {
                worktree_hash: "wtAhash".to_string(),
                branch: "develop".to_string(),
                path: PathBuf::from("/abs/wtA"),
            },
            status_payload: Ok(serde_json::json!({
                "status": {
                    "issues": {"healthy": true, "document_count": 100, "reason": "ready"},
                    "specs": {"healthy": true, "document_count": 50, "reason": "ready"},
                    "memory": {"healthy": false, "repair_required": true, "reason": "manifest_missing", "document_count": 0},
                    "files": {"healthy": true, "document_count": 310, "reason": "ready"},
                    "files-docs": {"healthy": true, "document_count": 16, "reason": "ready"}
                }
            })),
        }];

        let view = build_aggregated_status_view("asset-hash-12", &probes);

        assert_eq!(view.state, ProjectIndexStatusState::RepairRequired);
        assert!(
            view.detail.contains("1 index scope(s)"),
            "unhealthy memory must be counted (SPEC-2805); detail: {}",
            view.detail
        );
    }

    #[test]
    fn build_aggregated_status_view_aggregates_scope_health_across_worktrees() {
        let probes = vec![
            WorktreeProbeOutcome {
                input: WorktreeProbeInput {
                    worktree_hash: "wtAhash".to_string(),
                    branch: "develop".to_string(),
                    path: PathBuf::from("/abs/wtA"),
                },
                status_payload: Ok(serde_json::json!({
                    "status": {
                        "issues": {"healthy": true, "document_count": 100, "reason": "ready"},
                        "specs": {"healthy": false, "repair_required": true, "reason": "count_mismatch", "document_count": 5},
                        "files": {"healthy": true, "document_count": 310, "reason": "ready"},
                        "files-docs": {"healthy": false, "repair_required": true, "reason": "manifest_missing", "document_count": 0}
                    }
                })),
            },
            WorktreeProbeOutcome {
                input: WorktreeProbeInput {
                    worktree_hash: "wtBhash".to_string(),
                    branch: "feature/x".to_string(),
                    path: PathBuf::from("/abs/wtB"),
                },
                status_payload: Ok(serde_json::json!({
                    "status": {
                        "files": {"healthy": false, "repair_required": true, "reason": "manifest_missing", "document_count": 0},
                        "files-docs": {"healthy": true, "document_count": 16, "reason": "ready"}
                    }
                })),
            },
        ];

        let view = build_aggregated_status_view("asset-hash-12", &probes);

        assert_eq!(view.state, ProjectIndexStatusState::RepairRequired);
        // 1 specs + 1 files-docs (wtA) + 1 files (wtB) = 3 unhealthy
        assert!(
            view.detail.contains("3 index scope(s)"),
            "detail: {}",
            view.detail
        );
        assert!(view.scopes.issues.as_ref().unwrap().healthy);
        assert!(!view.scopes.specs.as_ref().unwrap().healthy);
        assert_eq!(view.scopes.files.len(), 2);
        assert!(view.scopes.files["wtAhash"].healthy);
        assert!(!view.scopes.files["wtBhash"].healthy);
        assert!(!view.scopes.files_docs["wtAhash"].healthy);
        assert!(view.scopes.files_docs["wtBhash"].healthy);
        assert_eq!(view.worktrees.len(), 2);
        assert_eq!(view.worktrees["wtAhash"].branch, "develop");
        assert_eq!(view.worktrees["wtBhash"].path, "/abs/wtB");
    }

    #[test]
    fn current_worktree_probe_inputs_exclude_inactive_worktrees() {
        let current = tempfile::tempdir().expect("current worktree");
        let inactive = tempfile::tempdir().expect("inactive worktree");
        let inputs = vec![
            WorktreeProbeInput {
                worktree_hash: "current-hash".to_string(),
                branch: "develop".to_string(),
                path: current.path().to_path_buf(),
            },
            WorktreeProbeInput {
                worktree_hash: "inactive-hash".to_string(),
                branch: "feature/inactive".to_string(),
                path: inactive.path().to_path_buf(),
            },
        ];

        let filtered = collect_current_worktree_probe_inputs(inputs, current.path());

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].worktree_hash, "current-hash");
        assert_eq!(filtered[0].branch, "develop");
    }

    #[test]
    fn current_worktree_probe_selection_without_match_does_not_fall_back_to_all_worktrees() {
        let current = tempfile::tempdir().expect("current worktree");
        let inactive_a = tempfile::tempdir().expect("inactive worktree a");
        let inactive_b = tempfile::tempdir().expect("inactive worktree b");
        let inputs = vec![
            WorktreeProbeInput {
                worktree_hash: "inactive-a".to_string(),
                branch: "feature/a".to_string(),
                path: inactive_a.path().to_path_buf(),
            },
            WorktreeProbeInput {
                worktree_hash: "inactive-b".to_string(),
                branch: "feature/b".to_string(),
                path: inactive_b.path().to_path_buf(),
            },
        ];

        let selection = select_probe_inputs_for_scope(
            inputs,
            StatusProbeScope::CurrentWorktree,
            Some(current.path()),
            32,
        );

        assert!(selection.inputs.is_empty());
        assert_eq!(
            selection.coverage.scope,
            ProjectIndexStatusCoverageScope::CurrentWorktree
        );
        assert_eq!(selection.coverage.probed_worktrees, 0);
        assert_eq!(selection.coverage.total_worktrees, 2);
        assert!(!selection.coverage.truncated);
    }

    #[test]
    fn all_worktree_probe_selection_is_batch_limited_and_reports_partial_coverage() {
        let inputs: Vec<WorktreeProbeInput> = (0..5)
            .map(|index| WorktreeProbeInput {
                worktree_hash: format!("wt-{index}"),
                branch: format!("branch-{index}"),
                path: PathBuf::from(format!("/tmp/worktree-{index}")),
            })
            .collect();

        let selection =
            select_probe_inputs_for_scope(inputs, StatusProbeScope::AllWorktrees, None, 3);

        assert_eq!(selection.inputs.len(), 3);
        assert_eq!(
            selection.coverage.scope,
            ProjectIndexStatusCoverageScope::PartialAllWorktrees
        );
        assert_eq!(selection.coverage.probed_worktrees, 3);
        assert_eq!(selection.coverage.total_worktrees, 5);
        assert!(selection.coverage.truncated);
    }

    #[test]
    fn aggregate_uses_test_fixture_when_env_var_set() {
        // SPEC-1939 T-IDX-109/110 follow-up scaffolding: when
        // GWT_INDEX_TEST_FIXTURE is set, the aggregator returns the parsed
        // fixture verbatim. Playwright e2e drives gwt with this env var so
        // tests can assert badge transitions deterministically without
        // running real Python.
        let _lock = GWT_INDEX_TEST_FIXTURE_ENV_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture_path = write_status_fixture(
            temp.path(),
            ProjectIndexStatusState::RepairRequired,
            "fixture-driven repair_required",
        );
        let _fixture_env = FixtureEnvGuard::set(&fixture_path);
        let view = aggregate_project_index_status_for_path(temp.path());

        assert_eq!(view.state, ProjectIndexStatusState::RepairRequired);
        assert_eq!(view.detail, "fixture-driven repair_required");
        assert!(view.scopes.specs.is_some());
    }

    #[test]
    fn project_index_status_uses_test_fixture_when_env_var_set() {
        let _lock = GWT_INDEX_TEST_FIXTURE_ENV_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture_path = write_status_fixture(
            temp.path(),
            ProjectIndexStatusState::RepairRequired,
            "fixture-driven repair_required",
        );
        let _fixture_env = FixtureEnvGuard::set(&fixture_path);

        let view = project_index_status_for_path(temp.path());

        assert_eq!(view.state, ProjectIndexStatusState::RepairRequired);
        assert_eq!(view.detail, "fixture-driven repair_required");
        assert!(view.scopes.specs.is_some());
    }

    #[test]
    fn bootstrap_skips_real_runner_when_test_fixture_env_var_set() {
        // The Playwright e2e workflow launches the GUI with fixture-backed
        // status and waits for the embedded server URL. The synchronous
        // startup bootstrap must therefore bypass the real Python runner
        // before the GUI server is started.
        let _lock = GWT_INDEX_TEST_FIXTURE_ENV_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).expect("create repo");
        init_git_repo_with_origin(&repo);
        let fixture_path = write_status_fixture(
            temp.path(),
            ProjectIndexStatusState::RepairRequired,
            "fixture-driven repair_required",
        );
        let _fixture_env = FixtureEnvGuard::set(&fixture_path);

        bootstrap_project_index_for_path_with(
            &repo,
            &temp.path().join("index"),
            &PanicRunnerSpawner,
        )
        .expect("fixture-backed bootstrap");
    }

    #[test]
    fn playwright_fixtures_deserialize_into_project_index_status_view() {
        // SPEC-1939 T-IDX-109/110 Playwright e2e: pin the fixture format so
        // a regression in `ProjectIndexStatusView` deserialization is
        // caught before CI shells out a misshapen fixture into the GUI.
        for relative in [
            "../gwt/playwright/fixtures/index-status-repair-required.json",
            "../gwt/playwright/fixtures/index-status-error.json",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
            let payload = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("read fixture {}: {err}", path.display()));
            let view: ProjectIndexStatusView = serde_json::from_str(&payload)
                .unwrap_or_else(|err| panic!("parse fixture {}: {err}", path.display()));
            assert!(
                matches!(
                    view.state,
                    ProjectIndexStatusState::RepairRequired | ProjectIndexStatusState::Error
                ),
                "fixture {} must seed a non-trivial badge state",
                path.display()
            );
        }
    }

    #[test]
    fn aggregate_test_fixture_invalid_json_surfaces_error_state() {
        let _lock = GWT_INDEX_TEST_FIXTURE_ENV_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture_path = temp.path().join("status.json");
        std::fs::write(&fixture_path, "{ this is not json }").expect("write fixture");

        let _fixture_env = FixtureEnvGuard::set(&fixture_path);
        let view = aggregate_project_index_status_for_path(temp.path());

        assert_eq!(view.state, ProjectIndexStatusState::Error);
        assert!(view.detail.contains("parse error"));
    }

    #[test]
    fn aggregated_status_cache_returns_cached_value_within_ttl() {
        let cache = AggregatedStatusCache::new(Duration::from_secs(60));
        let temp = tempfile::tempdir().expect("tempdir");
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let value1 = {
            let calls = calls.clone();
            cache.get_or_compute(temp.path(), |_| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "first")
            })
        };
        let value2 = {
            let calls = calls.clone();
            cache.get_or_compute(temp.path(), |_| {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "second")
            })
        };

        assert_eq!(value1.detail, "first");
        assert_eq!(value2.detail, "first", "second call should be cached");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn aggregated_status_cache_recomputes_after_invalidate() {
        let cache = AggregatedStatusCache::new(Duration::from_secs(60));
        let temp = tempfile::tempdir().expect("tempdir");

        let _initial = cache.get_or_compute(temp.path(), |_| {
            ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "initial")
        });
        cache.invalidate(temp.path());
        let recomputed = cache.get_or_compute(temp.path(), |_| {
            ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "recomputed")
        });

        assert_eq!(recomputed.detail, "recomputed");
    }

    #[test]
    fn aggregated_status_cache_recomputes_after_ttl_expires() {
        let cache = AggregatedStatusCache::new(Duration::from_millis(0));
        let temp = tempfile::tempdir().expect("tempdir");

        let _initial = cache.get_or_compute(temp.path(), |_| {
            ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "initial")
        });
        // Sleep 1ms to ensure elapsed > ttl=0 in all environments.
        std::thread::sleep(Duration::from_millis(1));
        let next = cache.get_or_compute(temp.path(), |_| {
            ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "next")
        });

        assert_eq!(next.detail, "next");
    }

    #[test]
    fn build_aggregated_status_view_reports_error_when_probe_fails() {
        let probes = vec![WorktreeProbeOutcome {
            input: WorktreeProbeInput {
                worktree_hash: "wtAhash".to_string(),
                branch: "develop".to_string(),
                path: PathBuf::from("/abs/wtA"),
            },
            status_payload: Err("runner exited 2".to_string()),
        }];

        let view = build_aggregated_status_view("asset-hash-12", &probes);

        assert_eq!(view.state, ProjectIndexStatusState::Error);
        assert!(
            view.detail.contains("runner exited 2"),
            "detail: {}",
            view.detail
        );
        assert_eq!(view.worktrees.len(), 1);
    }

    type FakeRebuildSpawnerCalls =
        std::sync::Arc<std::sync::Mutex<Vec<(IndexRebuildScope, Option<String>)>>>;

    struct FakeRebuildSpawner {
        calls: FakeRebuildSpawnerCalls,
        fail_at: Option<usize>,
    }

    impl IndexRebuildSpawner for FakeRebuildSpawner {
        fn rebuild(
            &self,
            _project_root: &Path,
            scope: IndexRebuildScope,
            worktree_hash: Option<&str>,
        ) -> Result<(), String> {
            let mut calls = self.calls.lock().unwrap();
            let idx = calls.len();
            calls.push((scope, worktree_hash.map(String::from)));
            if Some(idx) == self.fail_at {
                Err("synthetic failure".to_string())
            } else {
                Ok(())
            }
        }
    }

    fn unhealthy_status_with_specs_and_files_wt_a() -> ProjectIndexStatusView {
        let mut scopes = ProjectIndexScopes {
            specs: Some(ScopeHealthView::unhealthy("count_mismatch")),
            ..Default::default()
        };
        scopes.files.insert(
            "wtAhash".to_string(),
            ScopeHealthView::unhealthy("manifest_missing"),
        );
        ProjectIndexStatusView {
            state: ProjectIndexStatusState::RepairRequired,
            detail: "2 index scope(s) require repair".to_string(),
            repair_started_at: None,
            progress: None,
            scopes,
            worktrees: BTreeMap::new(),
            coverage: None,
        }
    }

    #[test]
    fn startup_auto_repair_targets_only_current_worktree_file_scopes() {
        let current = tempfile::tempdir().expect("current worktree");
        let inactive = tempfile::tempdir().expect("inactive worktree");
        let current_hash = compute_worktree_hash(current.path())
            .expect("current hash")
            .to_string();
        let inactive_hash = compute_worktree_hash(inactive.path())
            .expect("inactive hash")
            .to_string();

        let mut scopes = ProjectIndexScopes {
            issues: Some(ScopeHealthView::unhealthy("missing")),
            specs: Some(ScopeHealthView::unhealthy("count_mismatch")),
            ..Default::default()
        };
        scopes.files.insert(
            current_hash.clone(),
            ScopeHealthView::unhealthy("manifest_missing"),
        );
        scopes.files_docs.insert(
            current_hash.clone(),
            ScopeHealthView::unhealthy("collection_missing"),
        );
        scopes.files.insert(
            inactive_hash.clone(),
            ScopeHealthView::unhealthy("manifest_missing"),
        );
        scopes.files_docs.insert(
            inactive_hash,
            ScopeHealthView::unhealthy("collection_missing"),
        );

        let targets = collect_unhealthy_rebuild_targets_for_project_root(&scopes, current.path());

        assert_eq!(
            targets,
            vec![
                (IndexRebuildScope::Issues, None),
                (IndexRebuildScope::Specs, None),
                (IndexRebuildScope::Files, Some(current_hash.clone())),
                (IndexRebuildScope::FilesDocs, Some(current_hash)),
            ]
        );
    }

    #[test]
    fn auto_repair_orchestrator_does_nothing_when_state_is_ready() {
        let initial = ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "ready");
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_for_sink = recorded.clone();
        let handle = auto_repair_unhealthy_scopes(
            PathBuf::from("/tmp/never"),
            &initial,
            FakeRebuildSpawner {
                calls: calls.clone(),
                fail_at: None,
            },
            |_| ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "still ready"),
            move |view| recorded_for_sink.lock().unwrap().push(view),
        );

        assert!(handle.is_none(), "no thread should be spawned");
        assert!(calls.lock().unwrap().is_empty());
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[test]
    fn auto_repair_orchestrator_emits_repairing_progression_and_finishes_ready() {
        let initial = unhealthy_status_with_specs_and_files_wt_a();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_for_sink = recorded.clone();

        let handle = auto_repair_unhealthy_scopes(
            PathBuf::from("/tmp/test-project"),
            &initial,
            FakeRebuildSpawner {
                calls: calls.clone(),
                fail_at: None,
            },
            |_| ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "ready after rebuild"),
            move |view| recorded_for_sink.lock().unwrap().push(view),
        )
        .expect("orchestrator thread spawned");
        handle.join().expect("orchestrator thread joined");

        let calls_snapshot = calls.lock().unwrap().clone();
        assert_eq!(
            calls_snapshot,
            vec![
                (IndexRebuildScope::Specs, None),
                (IndexRebuildScope::Files, Some("wtAhash".to_string())),
            ]
        );

        let events = recorded.lock().unwrap().clone();
        assert_eq!(events.len(), 4, "expect 4 events: 0/2, 1/2, 2/2, ready");
        assert_eq!(events[0].state, ProjectIndexStatusState::Repairing);
        assert_eq!(events[0].progress.unwrap().scopes_done, 0);
        assert_eq!(events[0].progress.unwrap().scopes_total, 2);
        assert_eq!(events[1].state, ProjectIndexStatusState::Repairing);
        assert_eq!(events[1].progress.unwrap().scopes_done, 1);
        assert_eq!(events[2].state, ProjectIndexStatusState::Repairing);
        assert_eq!(events[2].progress.unwrap().scopes_done, 2);
        assert_eq!(events[3].state, ProjectIndexStatusState::Ready);
        assert_eq!(events[3].detail, "ready after rebuild");
    }

    #[test]
    fn auto_repair_orchestrator_stops_at_first_failure_with_error_state() {
        let initial = unhealthy_status_with_specs_and_files_wt_a();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_for_sink = recorded.clone();

        let handle = auto_repair_unhealthy_scopes(
            PathBuf::from("/tmp/test-project"),
            &initial,
            FakeRebuildSpawner {
                calls: calls.clone(),
                fail_at: Some(0),
            },
            |_| ProjectIndexStatusView::new(ProjectIndexStatusState::Ready, "should not be used"),
            move |view| recorded_for_sink.lock().unwrap().push(view),
        )
        .expect("orchestrator thread spawned");
        handle.join().expect("orchestrator thread joined");

        // First scope failed → exactly one call recorded.
        let calls_snapshot = calls.lock().unwrap().clone();
        assert_eq!(calls_snapshot.len(), 1);

        let events = recorded.lock().unwrap().clone();
        assert_eq!(events.len(), 2, "expect Repairing(0/N) then Error");
        assert_eq!(events[0].state, ProjectIndexStatusState::Repairing);
        assert_eq!(events[1].state, ProjectIndexStatusState::Error);
        assert!(events[1].detail.contains("synthetic failure"));
    }

    #[test]
    fn issue_index_source_mismatch_requires_startup_rebuild() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_hash =
            gwt_core::repo_hash::compute_repo_hash("https://github.com/example/gwt.git");
        let cache_root = temp.path().join("issue-cache");
        let snapshot = gwt_github::IssueSnapshot {
            number: gwt_github::IssueNumber(2867),
            title: "Recent Projects".to_string(),
            body: "workspace home".to_string(),
            labels: vec!["bug".to_string()],
            state: gwt_github::IssueState::Closed,
            updated_at: gwt_github::UpdatedAt::new("2026-05-23T00:00:00Z"),
            comments: vec![],
        };
        gwt_github::Cache::new(cache_root.clone())
            .write_snapshot(&snapshot)
            .expect("write cache");

        let index_root = temp.path().join("index");
        let issues_dir = index_root.join(repo_hash.as_str()).join("issues");
        std::fs::create_dir_all(&issues_dir).expect("issues dir");
        std::fs::write(
            issues_dir.join("meta.json"),
            serde_json::json!({
                "schema_version": 1,
                "last_full_refresh": chrono::Utc::now().to_rfc3339(),
                "ttl_minutes": 15,
                "source_cache_fingerprint": "stale",
                "source_document_count": 1,
            })
            .to_string(),
        )
        .expect("write meta");

        assert!(
            issue_index_needs_rebuild_for_cache(&index_root, &repo_hash, &cache_root)
                .expect("rebuild decision"),
            "startup must rebuild issues when source cache fingerprint differs from index meta",
        );
    }

    #[test]
    fn project_index_status_view_serializes_aggregated_scopes_and_worktrees() {
        let mut scopes = ProjectIndexScopes {
            issues: Some(ScopeHealthView::ready(42)),
            specs: Some(ScopeHealthView::unhealthy("count_mismatch")),
            ..Default::default()
        };
        scopes
            .files
            .insert("wtAhash".to_string(), ScopeHealthView::ready(310));
        scopes.files_docs.insert(
            "wtBhash".to_string(),
            ScopeHealthView::unhealthy("manifest_missing"),
        );

        let mut worktrees: BTreeMap<String, WorktreeMeta> = BTreeMap::new();
        worktrees.insert(
            "wtAhash".to_string(),
            WorktreeMeta {
                branch: "develop".to_string(),
                path: "/abs/wtA".to_string(),
            },
        );
        worktrees.insert(
            "wtBhash".to_string(),
            WorktreeMeta {
                branch: "feature/x".to_string(),
                path: "/abs/wtB".to_string(),
            },
        );

        let view = ProjectIndexStatusView {
            state: ProjectIndexStatusState::Repairing,
            detail: "rebuilding 1/4".to_string(),
            repair_started_at: None,
            progress: Some(RebuildProgress {
                scopes_done: 1,
                scopes_total: 4,
            }),
            scopes,
            worktrees,
            coverage: None,
        };

        let payload = serde_json::to_value(&view).expect("serialize aggregated view");

        assert_eq!(payload["state"], "repairing");
        assert_eq!(payload["scopes"]["issues"]["healthy"], true);
        assert_eq!(payload["scopes"]["issues"]["document_count"], 42);
        assert_eq!(payload["scopes"]["specs"]["repair_required"], true);
        assert_eq!(payload["scopes"]["specs"]["reason"], "count_mismatch");
        assert_eq!(payload["scopes"]["files"]["wtAhash"]["healthy"], true);
        assert_eq!(
            payload["scopes"]["files-docs"]["wtBhash"]["reason"],
            "manifest_missing"
        );
        assert_eq!(payload["worktrees"]["wtAhash"]["branch"], "develop");
        assert_eq!(payload["worktrees"]["wtBhash"]["path"], "/abs/wtB");
    }

    #[test]
    fn run_coordinated_index_job_runs_the_owner_build() {
        let run = run_coordinated_index_job(
            "ownertestrepo001",
            "files",
            Some("wt001"),
            JobPriority::Background,
            || Ok(BuildStep::Done(7)),
        )
        .expect("owner build succeeds");
        match run {
            CoordinatedRun::Ran(value) => assert_eq!(value, 7),
            CoordinatedRun::Coalesced => panic!("no concurrent job exists"),
        }
    }

    #[test]
    fn run_coordinated_index_job_propagates_build_failures() {
        let error = run_coordinated_index_job::<()>(
            "failtestrepo0001",
            "issues",
            None,
            JobPriority::Background,
            || Err("boom".to_string()),
        )
        .expect_err("build failure propagates");
        assert_eq!(error, "boom");
    }

    #[test]
    fn run_coordinated_index_job_resumes_after_a_yield() {
        let mut calls = 0;
        let run = run_coordinated_index_job(
            "yieldtestrepo001",
            "files",
            Some("wt002"),
            JobPriority::Background,
            || {
                calls += 1;
                if calls == 1 {
                    Ok(BuildStep::Yielded)
                } else {
                    Ok(BuildStep::Done(()))
                }
            },
        )
        .expect("yielded build resumes");
        assert!(matches!(run, CoordinatedRun::Ran(())));
        assert_eq!(calls, 2, "the build must re-run after releasing the lease");
    }

    #[test]
    fn run_coordinated_index_job_coalesces_into_a_running_equivalent_job() {
        use gwt_core::index_coordinator::{IndexCoordinator, JobAdmission, JobOutcome, TargetKey};
        let tmp = tempfile::tempdir().expect("tempdir");
        let started = tmp.path().join("owner-started");
        let release = tmp.path().join("release-owner");
        let started_for_thread = started.clone();
        let release_for_thread = release.clone();

        let owner = std::thread::spawn(move || {
            let coordinator = IndexCoordinator::open_default().expect("open coordinator");
            let key = TargetKey::repo_shared("coalescetestrepo", "specs");
            let guard = match coordinator
                .request_job(
                    &key,
                    JobPriority::Background,
                    std::time::Duration::from_secs(5),
                )
                .expect("request job")
            {
                JobAdmission::Owner(guard) => guard,
                JobAdmission::Joined(_) => panic!("owner thread must win the target"),
            };
            std::fs::write(&started_for_thread, b"started").expect("write marker");
            let deadline = Instant::now() + Duration::from_secs(10);
            while !release_for_thread.exists() {
                assert!(Instant::now() < deadline, "release signal never arrived");
                std::thread::sleep(Duration::from_millis(10));
            }
            guard.complete(JobOutcome::Completed).expect("complete");
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while !started.exists() {
            assert!(Instant::now() < deadline, "owner never started");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(&release, b"go").expect("write release");
        let run = run_coordinated_index_job::<()>(
            "coalescetestrepo",
            "specs",
            None,
            JobPriority::Background,
            || panic!("a coalesced caller must not run its own build"),
        )
        .expect("coalesced join succeeds");
        assert!(matches!(run, CoordinatedRun::Coalesced));
        owner.join().expect("owner thread");
    }
}
