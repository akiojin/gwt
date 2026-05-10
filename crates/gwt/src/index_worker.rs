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
        runtime::{
            reconcile_repo, refresh_issues_if_stale, PythonRunnerSpawner, ReconcileOptions,
            RefreshIssuesOptions, RunnerSpawner,
        },
    },
    paths::{gwt_project_index_venv_dir, gwt_runtime_runner_path},
    repo_hash::RepoHash,
    worktree_hash::compute_worktree_hash,
};
use serde::Serialize;

/// Determine `RepoHash` for the given repository root by shelling out to
/// `git remote get-url origin`. Returns `None` if no origin is configured.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    gwt_core::repo_hash::detect_repo_hash(repo_root)
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
        runner_script: gwt_runtime_runner_path(),
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
    Files,
    #[serde(rename = "files-docs")]
    FilesDocs,
}

impl IndexRebuildScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Issues => "issues",
            Self::Specs => "specs",
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
        }
    }
}

pub fn project_index_status_for_path(project_root: &Path) -> ProjectIndexStatusView {
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
    match aggregate_project_index_status_for_path_inner(project_root) {
        Ok(status) => status,
        Err(error) => ProjectIndexStatusView::new(ProjectIndexStatusState::Error, error),
    }
}

fn aggregate_project_index_status_for_path_inner(
    project_root: &Path,
) -> Result<ProjectIndexStatusView, String> {
    let Some(repo_root) = resolve_git_worktree_root(project_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No git worktree detected",
        ));
    };
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No origin remote configured",
        ));
    };
    let runtime_started = Instant::now();
    let report =
        gwt_core::runtime::ensure_project_index_runtime().map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %project_root.display(),
        elapsed_ms = runtime_started.elapsed().as_millis() as u64,
        "project index runtime ensured for aggregated status"
    );

    let inputs = list_worktree_probe_inputs(&repo_root)?;
    let mut probes: Vec<WorktreeProbeOutcome> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let payload = probe_worktree_status(&repo_root, &repo_hash, &input.worktree_hash);
        probes.push(WorktreeProbeOutcome {
            input,
            status_payload: payload,
        });
    }

    Ok(build_aggregated_status_view(
        report.runner_hash.as_str(),
        &probes,
    ))
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

    let mut inputs = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().chain(std::iter::once("")) {
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(path) = current_path.take() {
                push_worktree_probe_input(&mut inputs, path, current_branch.take())?;
            }
            current_path = Some(canonicalize_path(PathBuf::from(rest)));
        } else if let Some(branch) = line.strip_prefix("branch ") {
            current_branch = Some(branch.trim_start_matches("refs/heads/").to_string());
        } else if line.is_empty() {
            if let Some(path) = current_path.take() {
                push_worktree_probe_input(&mut inputs, path, current_branch.take())?;
            }
        }
    }

    Ok(inputs)
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

fn probe_worktree_status(
    repo_root: &Path,
    repo_hash: &RepoHash,
    worktree_hash: &str,
) -> Result<serde_json::Value, String> {
    let runner_started = Instant::now();
    let output = gwt_core::process::hidden_command(project_index_python_path())
        .arg(gwt_runtime_runner_path())
        .arg("--action")
        .arg("status")
        .arg("--repo-hash")
        .arg(repo_hash.as_str())
        .arg("--worktree-hash")
        .arg(worktree_hash)
        .current_dir(repo_root)
        .output()
        .map_err(|err| format!("run project index status: {err}"))?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        worktree_hash,
        elapsed_ms = runner_started.elapsed().as_millis() as u64,
        exit_status = %output.status,
        "project index status runner completed for worktree"
    );
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!("runner exit {}: {detail}", output.status));
    }
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

/// Production rebuild runner that resolves an `IndexContext` for the
/// requested `(project_root, worktree_hash?)` and invokes the runner via the
/// same path used by `gwt index rebuild`. Used by both the auto-rebuild
/// orchestrator and the per-cell IPC entry, so concurrent CLI/GUI calls
/// funnel through the same `.lock` sentinel.
pub fn default_rebuild_runner(
    project_root: &Path,
    scope: IndexRebuildScope,
    worktree_hash: Option<&str>,
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
    let output = run_runner_rebuild(&ctx, action).map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(format_runner_failure(&output));
    }
    Ok(())
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
    let Some(repo_root) = resolve_git_worktree_root(project_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No git worktree detected",
        ));
    };
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No origin remote configured",
        ));
    };
    let worktree_hash =
        compute_worktree_hash(&repo_root).map_err(|err| format!("compute worktree hash: {err}"))?;
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
        .arg(gwt_runtime_runner_path())
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
    let Some(repo_root) = resolve_git_worktree_root(project_root) else {
        return Ok(());
    };
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

    let refresh = RefreshIssuesOptions {
        index_root: index_root.to_path_buf(),
        repo_hash,
        project_root: repo_root.clone(),
        ttl: Duration::from_secs(15 * 60),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    let refresh_started = Instant::now();
    runtime
        .block_on(refresh_issues_if_stale(&refresh, spawner))
        .map(|decision| {
            tracing::info!(
                target: "gwt::index",
                project_root = %repo_root.display(),
                elapsed_ms = refresh_started.elapsed().as_millis() as u64,
                decision = ?decision,
                "project index issue refresh checked"
            );
        })
        .map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = bootstrap_started.elapsed().as_millis() as u64,
        "project index bootstrap helper completed"
    );

    Ok(())
}

fn resolve_git_worktree_root(path: &Path) -> Option<PathBuf> {
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

fn list_git_worktree_paths(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project_root)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let mut worktrees = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            worktrees.push(canonicalize_path(PathBuf::from(path)));
        }
    }

    if worktrees.is_empty() {
        worktrees.push(canonicalize_path(project_root.to_path_buf()));
    }
    Ok(worktrees)
}

fn canonicalize_path(path: PathBuf) -> PathBuf {
    dunce::canonicalize(&path).unwrap_or(path)
}

pub(crate) fn project_index_python_path() -> PathBuf {
    let venv = gwt_project_index_venv_dir();
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python3")
    }
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
        assert_eq!(view.scopes.files.len(), 1);
        assert_eq!(view.scopes.files_docs.len(), 1);
        assert_eq!(view.worktrees.len(), 1);
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
        }
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
}
