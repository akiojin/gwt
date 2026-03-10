//! Project structure index commands backed by ChromaDB (Python runtime).

use crate::commands::project::{resolve_project_root, resolve_repo_path_for_project_root};
use gwt_core::process::command_os;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::warn;
use uuid::Uuid;

const CHROMA_VENV_DIR: &str = "chroma-venv";
const CHROMA_RUNTIME_PIP_DEPS: &[&str] = &["chromadb"];
const CHROMA_HELPER_SCRIPT: &str = include_str!("../python/chroma_index_runner.py");

static CHROMA_RUNTIME_PROBE: Mutex<Option<Result<(), String>>> = Mutex::new(None);
static PROJECT_INDEX_RECOVERY_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    OnceLock::new();

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChromaRunnerResponse {
    ok: bool,
    error: Option<String>,
    #[serde(default)]
    files_indexed: Option<u64>,
    #[serde(default)]
    issues_indexed: Option<u64>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    results: Option<Vec<SearchResultItem>>,
    #[serde(default)]
    issue_results: Option<Vec<GitHubIssueSearchResult>>,
    #[serde(default)]
    indexed: Option<bool>,
    #[serde(default)]
    total_files: Option<u64>,
    #[serde(default)]
    db_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultItem {
    pub path: String,
    pub description: String,
    pub distance: Option<f64>,
    pub file_type: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRuntimeSetupResult {
    pub ready: bool,
    pub installed: bool,
    pub python_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProjectResult {
    pub files_indexed: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatusResult {
    pub indexed: bool,
    pub total_files: u64,
    pub db_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssueSearchResult {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub labels: Vec<String>,
    pub distance: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexIssuesResult {
    pub issues_indexed: u64,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Runtime helpers
// ---------------------------------------------------------------------------

fn gwt_runtime_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    Ok(home.join(".gwt").join("runtime"))
}

fn chroma_venv_dir() -> Result<PathBuf, String> {
    Ok(gwt_runtime_dir()?.join(CHROMA_VENV_DIR))
}

fn chroma_python_path(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python3")
    }
}

fn system_python_candidates() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "python3.13",
            "python3.12",
            "python3.11",
            "python3",
            "py",
            "python",
        ]
    }

    #[cfg(not(windows))]
    {
        &[
            "python3.13",
            "python3.12",
            "python3.11",
            "python3",
            "python",
        ]
    }
}

fn is_windows_store_python_alias(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let normalized = path
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        normalized.contains("\\appdata\\local\\microsoft\\windowsapps\\")
            && file_name.starts_with("python")
            && file_name.ends_with(".exe")
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn can_execute_python(path: &Path) -> bool {
    match command_os(path)
        .arg("-c")
        .arg("import sys; print(sys.version_info[0])")
        .output()
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim() == "3"
        }
        _ => false,
    }
}

fn find_system_python() -> Result<PathBuf, String> {
    for candidate in system_python_candidates() {
        if let Ok(path) = which::which(candidate) {
            if can_execute_python(&path) {
                return Ok(path);
            }

            if is_windows_store_python_alias(&path) {
                continue;
            }
        }
    }

    Err(format!(
        "Python runtime not found (checked {})",
        system_python_candidates().join("/")
    ))
}

fn find_python_binary() -> Result<PathBuf, String> {
    let venv_dir = chroma_venv_dir()?;
    let managed = chroma_python_path(&venv_dir);
    if managed.is_file() {
        Ok(managed)
    } else {
        find_system_python()
    }
}

fn ensure_chroma_runner_script() -> Result<PathBuf, String> {
    let runtime_dir = gwt_runtime_dir()?;
    fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("Failed to create runtime directory: {e}"))?;

    let script_path = runtime_dir.join("chroma_index_runner.py");

    let needs_write = match fs::read_to_string(&script_path) {
        Ok(existing) => existing != CHROMA_HELPER_SCRIPT,
        Err(_) => true,
    };

    if needs_write {
        let mut file = fs::File::create(&script_path)
            .map_err(|e| format!("Failed to create chroma helper script: {e}"))?;
        file.write_all(CHROMA_HELPER_SCRIPT.as_bytes())
            .map_err(|e| format!("Failed to write chroma helper script: {e}"))?;
        file.flush()
            .map_err(|e| format!("Failed to flush chroma helper script: {e}"))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&script_path)
                .map_err(|e| format!("Failed to stat chroma helper script: {e}"))?
                .permissions();
            perm.set_mode(0o700);
            if let Err(e) = fs::set_permissions(&script_path, perm) {
                warn!(
                    category = "project_index",
                    path = %script_path.display(),
                    error = %e,
                    "Failed to set permissions on chroma helper script"
                );
            }
        }
    }

    Ok(script_path)
}

fn run_chroma_runner(
    python: &Path,
    action: &str,
    project_root: Option<&str>,
    db_path: Option<&str>,
    query: Option<&str>,
    n_results: Option<u32>,
) -> Result<ChromaRunnerResponse, String> {
    let script = ensure_chroma_runner_script()?;

    let mut cmd = command_os(python);
    cmd.arg(&script).arg("--action").arg(action);

    if let Some(root) = project_root {
        cmd.arg("--project-root").arg(root);
    }
    if let Some(db) = db_path {
        cmd.arg("--db-path").arg(db);
    }
    if let Some(q) = query {
        cmd.arg("--query").arg(q);
    }
    if let Some(n) = n_results {
        cmd.arg("--n-results").arg(n.to_string());
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run chroma helper script: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let mut exit_detail = format!("{}", output.status);
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = output.status.signal() {
                exit_detail.push_str(&format!(", signal={signal}"));
            }
        }
        return Err(format!(
            "Chroma helper failed (status={}): {}{}",
            exit_detail,
            if stderr.is_empty() {
                "<no stderr>"
            } else {
                &stderr
            },
            if stdout.is_empty() {
                String::new()
            } else {
                format!("; stdout={stdout}")
            }
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("Chroma helper returned non UTF-8 stdout: {e}"))?;
    let parsed: ChromaRunnerResponse = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("Failed to parse chroma helper response: {e}"))?;

    if parsed.ok {
        Ok(parsed)
    } else {
        Err(parsed
            .error
            .unwrap_or_else(|| "Chroma helper returned failure without error".to_string()))
    }
}

fn run_command_with_output(mut cmd: std::process::Command, context: &str) -> Result<(), String> {
    let output = cmd
        .output()
        .map_err(|e| format!("{context}: failed to start command: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(format!(
        "{context}: command failed (status={}): {}{}",
        output.status,
        if stderr.is_empty() {
            "<no stderr>"
        } else {
            &stderr
        },
        if stdout.is_empty() {
            String::new()
        } else {
            format!("; stdout={stdout}")
        }
    ))
}

fn clear_runtime_probe_cache() {
    if let Ok(mut guard) = CHROMA_RUNTIME_PROBE.lock() {
        *guard = None;
    }
}

fn probe_chroma_runtime() -> Result<(), String> {
    let python = find_python_binary()?;
    let _ = run_chroma_runner(&python, "probe", None, None, None, None)?;
    Ok(())
}

fn probe_chroma_runtime_cached() -> Result<(), String> {
    let mut guard = CHROMA_RUNTIME_PROBE
        .lock()
        .map_err(|_| "Failed to lock chroma runtime probe cache".to_string())?;
    if let Some(result) = guard.as_ref() {
        return result.clone();
    }
    let result = probe_chroma_runtime();
    *guard = Some(result.clone());
    result
}

fn ensure_chroma_runtime_sync() -> Result<IndexRuntimeSetupResult, String> {
    let runtime_dir = gwt_runtime_dir()?;
    fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("Failed to create runtime directory: {e}"))?;

    let venv_dir = chroma_venv_dir()?;
    let managed_python = chroma_python_path(&venv_dir);
    let mut installed = false;

    if !managed_python.is_file() {
        // Invalidate probe cache because find_python_binary() target is about to change.
        clear_runtime_probe_cache();
        let bootstrap_python = find_system_python()?;
        let mut cmd = command_os(&bootstrap_python);
        cmd.arg("-m").arg("venv").arg(&venv_dir);
        run_command_with_output(cmd, "Failed to create chroma runtime virtual environment")?;
        installed = true;
    }

    // Only consider runtime "ready" when the managed runtime can probe successfully.
    if run_chroma_runner(&managed_python, "probe", None, None, None, None).is_ok() {
        return Ok(IndexRuntimeSetupResult {
            ready: true,
            installed,
            python_path: managed_python.to_string_lossy().to_string(),
        });
    }

    // Upgrade pip
    let mut pip_upgrade = command_os(&managed_python);
    pip_upgrade
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("--upgrade")
        .arg("pip")
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1");
    run_command_with_output(pip_upgrade, "Failed to update pip for chroma runtime")?;

    // Check if chromadb already works
    let probe_result = run_chroma_runner(&managed_python, "probe", None, None, None, None);
    if probe_result.is_err() {
        let mut install = command_os(&managed_python);
        install.arg("-m").arg("pip").arg("install").arg("--upgrade");
        for dep in CHROMA_RUNTIME_PIP_DEPS {
            install.arg(dep);
        }
        install.env("PIP_DISABLE_PIP_VERSION_CHECK", "1");
        run_command_with_output(install, "Failed to install chroma runtime dependencies")?;
        installed = true;
    }

    // Verify installation
    let _ = run_chroma_runner(&managed_python, "probe", None, None, None, None)?;
    clear_runtime_probe_cache();
    probe_chroma_runtime_cached()?;

    Ok(IndexRuntimeSetupResult {
        ready: true,
        installed,
        python_path: managed_python.to_string_lossy().to_string(),
    })
}

fn db_path_for_project(project_root: impl AsRef<Path>) -> PathBuf {
    project_root.as_ref().join(".gwt").join("index")
}

fn canonical_project_root_for_index(project_root: &Path) -> PathBuf {
    dunce::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf())
}

fn remove_empty_legacy_gwt_dir(legacy_db_path: &Path) {
    let Some(legacy_gwt_dir) = legacy_db_path.parent() else {
        return;
    };

    match fs::read_dir(legacy_gwt_dir) {
        Ok(entries) => {
            let mut entries = entries;
            if entries.next().is_none() {
                if let Err(error) = fs::remove_dir(legacy_gwt_dir) {
                    warn!(
                        category = "project_index",
                        path = %legacy_gwt_dir.display(),
                        error = %error,
                        "Failed to remove empty legacy .gwt directory after migrating project index"
                    );
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            warn!(
                category = "project_index",
                path = %legacy_gwt_dir.display(),
                error = %error,
                "Failed to inspect legacy .gwt directory after migrating project index"
            );
        }
    }
}

fn migrate_legacy_worktree_index(
    requested_root: &Path,
    normalized_root: &Path,
) -> Result<(), String> {
    if canonical_project_root_for_index(requested_root)
        == canonical_project_root_for_index(normalized_root)
    {
        return Ok(());
    }

    let legacy_db_path = db_path_for_project(requested_root);
    if !legacy_db_path.exists() {
        return Ok(());
    }

    let normalized_db_path = db_path_for_project(normalized_root);
    if normalized_db_path.exists() {
        warn!(
            category = "project_index",
            legacy_db_path = %legacy_db_path.display(),
            normalized_db_path = %normalized_db_path.display(),
            "Leaving legacy worktree project index in place because the normalized project root already has an index"
        );
        return Ok(());
    }

    let normalized_parent = normalized_db_path.parent().ok_or_else(|| {
        format!(
            "Normalized project index path has no parent: {}",
            normalized_db_path.display()
        )
    })?;
    fs::create_dir_all(normalized_parent).map_err(|error| {
        format!(
            "Failed to create normalized project index directory {}: {error}",
            normalized_parent.display()
        )
    })?;
    fs::rename(&legacy_db_path, &normalized_db_path).map_err(|error| {
        format!(
            "Failed to migrate legacy worktree project index {} -> {}: {error}",
            legacy_db_path.display(),
            normalized_db_path.display()
        )
    })?;

    tracing::info!(
        category = "project_index",
        legacy_db_path = %legacy_db_path.display(),
        normalized_db_path = %normalized_db_path.display(),
        "Migrated legacy worktree project index to normalized project root"
    );

    remove_empty_legacy_gwt_dir(&legacy_db_path);
    Ok(())
}

fn normalize_project_root_for_index(project_root: &str) -> Result<PathBuf, String> {
    let requested_root = PathBuf::from(project_root);
    let normalized_root = resolve_project_root(&requested_root);
    migrate_legacy_worktree_index(&requested_root, &normalized_root)?;
    Ok(normalized_root)
}

fn is_recoverable_chroma_db_error(error: &str) -> bool {
    let normalized = error.to_lowercase();
    normalized.contains("signal=11")
        || normalized.contains("sigsegv")
        || normalized.contains("segmentation fault")
        || normalized.contains("status=139")
}

fn quarantine_index_db(db_path: &Path) -> Result<PathBuf, String> {
    if !db_path.exists() {
        return Err(format!(
            "Index database does not exist at {}",
            db_path.to_string_lossy()
        ));
    }

    let parent = db_path.parent().ok_or_else(|| {
        format!(
            "Index database has no parent: {}",
            db_path.to_string_lossy()
        )
    })?;
    let backup_path = parent.join(format!("index.crashed-{}", Uuid::new_v4()));

    fs::rename(db_path, &backup_path).map_err(|e| {
        format!(
            "Failed to quarantine crashing index database {} -> {}: {e}",
            db_path.to_string_lossy(),
            backup_path.to_string_lossy()
        )
    })?;

    Ok(backup_path)
}

fn repo_path_for_issue_index(project_root: &str) -> Result<PathBuf, String> {
    resolve_repo_path_for_project_root(Path::new(project_root))
}

fn chroma_db_contains_collection(
    python: &Path,
    db_sqlite_path: &Path,
    collection_name: &str,
) -> Result<bool, String> {
    let output = command_os(python)
        .arg("-c")
        .arg(
            "import sqlite3, sys; \
             conn = sqlite3.connect(sys.argv[1]); \
             cur = conn.cursor(); \
             row = cur.execute('select 1 from collections where name = ? limit 1', (sys.argv[2],)).fetchone(); \
             conn.close(); \
             print('1' if row else '0')",
        )
        .arg(db_sqlite_path)
        .arg(collection_name)
        .output()
        .map_err(|e| format!("Failed to inspect Chroma sqlite metadata: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Failed to inspect Chroma sqlite metadata (status={}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
}

fn rebuild_issue_index(
    python: &Path,
    project_root: &str,
    db_path: &str,
    explicit_repo_path: Option<&str>,
) -> Result<ChromaRunnerResponse, String> {
    let repo_path = match explicit_repo_path {
        Some(path) => path.to_string(),
        None => repo_path_for_issue_index(project_root)?
            .to_string_lossy()
            .to_string(),
    };

    run_chroma_runner(
        python,
        "index-issues",
        Some(&repo_path),
        Some(db_path),
        None,
        None,
    )
}

fn project_index_recovery_lock(db_path: &Path) -> Arc<Mutex<()>> {
    let cache = PROJECT_INDEX_RECOVERY_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    guard
        .entry(db_path.to_string_lossy().to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn should_rebuild_issues_collection(
    action: &str,
    issues_collection_presence: Result<bool, String>,
) -> bool {
    if matches!(action, "index-issues" | "search-issues") {
        return true;
    }

    match issues_collection_presence {
        Ok(present) => present,
        Err(probe_err) => {
            warn!(
                category = "project_index",
                action = action,
                error = %probe_err,
                "Failed to inspect issues collection state before recovery; rebuilding issues index to avoid silent data loss"
            );
            true
        }
    }
}

fn run_project_index_action_with_crash_recovery(
    action: &str,
    project_root: &str,
    explicit_repo_path: Option<&str>,
    query: Option<&str>,
    n_results: Option<u32>,
) -> Result<ChromaRunnerResponse, String> {
    let python = find_python_binary()?;
    let project_root = normalize_project_root_for_index(project_root)?;
    let project_root = project_root.to_string_lossy().to_string();
    let db = db_path_for_project(&project_root);
    let db_path = db.to_string_lossy().to_string();

    let run_primary = || match action {
        "index" => run_chroma_runner(
            &python,
            "index",
            Some(&project_root),
            Some(&db_path),
            None,
            None,
        ),
        "search" => run_chroma_runner(&python, "search", None, Some(&db_path), query, n_results),
        "status" => run_chroma_runner(&python, "status", None, Some(&db_path), None, None),
        "index-issues" => rebuild_issue_index(&python, &project_root, &db_path, explicit_repo_path),
        "search-issues" => run_chroma_runner(
            &python,
            "search-issues",
            None,
            Some(&db_path),
            query,
            n_results,
        ),
        _ => Err(format!(
            "Unsupported Project Index action for crash recovery: {action}"
        )),
    };

    match run_primary() {
        Ok(resp) => Ok(resp),
        Err(err) if is_recoverable_chroma_db_error(&err) => {
            let recovery_lock = project_index_recovery_lock(&db);
            let _guard = recovery_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());

            let err = match run_primary() {
                Ok(resp) => return Ok(resp),
                Err(retry_err) if db.exists() && is_recoverable_chroma_db_error(&retry_err) => {
                    retry_err
                }
                Err(retry_err) => return Err(retry_err),
            };

            let issues_collection_presence =
                chroma_db_contains_collection(&python, &db.join("chroma.sqlite3"), "issues");
            let backup_path = quarantine_index_db(&db)?;
            warn!(
                category = "project_index",
                action = action,
                db_path = %db.display(),
                backup_path = %backup_path.display(),
                error = %err,
                "Recovered from crashing Project Index database by quarantining the persisted database"
            );

            let rebuilt_files = run_chroma_runner(
                &python,
                "index",
                Some(&project_root),
                Some(&db_path),
                None,
                None,
            )
            .map_err(|rebuild_err| {
                format!(
                    "Recovered crashing index at {} to {} but failed to rebuild files index: {}; original error: {}",
                    db.display(),
                    backup_path.display(),
                    rebuild_err,
                    err
                )
            })?;

            let needs_issue_rebuild =
                should_rebuild_issues_collection(action, issues_collection_presence);
            let rebuilt_issues = if needs_issue_rebuild {
                Some(
                    rebuild_issue_index(&python, &project_root, &db_path, explicit_repo_path)
                        .map_err(|rebuild_err| {
                            format!(
                                "Recovered crashing index at {} to {} and rebuilt files index, but failed to rebuild issues index: {}; original error: {}",
                                db.display(),
                                backup_path.display(),
                                rebuild_err,
                                err
                            )
                        })?,
                )
            } else {
                None
            };

            if matches!(action, "index" | "index-issues") {
                return Ok(if action == "index" {
                    rebuilt_files
                } else {
                    rebuilt_issues.expect("issues rebuild must exist for index-issues")
                });
            }

            run_primary().map_err(|retry_err| {
                format!(
                    "Recovered crashing index at {} to {} and rebuilt required collections, but retrying action '{}' failed: {}",
                    db.display(),
                    backup_path.display(),
                    action,
                    retry_err
                )
            })
        }
        Err(err) => Err(err),
    }
}

fn run_files_action_with_crash_recovery(
    action: &str,
    project_root: &str,
    query: Option<&str>,
    n_results: Option<u32>,
) -> Result<ChromaRunnerResponse, String> {
    run_project_index_action_with_crash_recovery(action, project_root, None, query, n_results)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ensure_index_runtime() -> Result<IndexRuntimeSetupResult, String> {
    tokio::task::spawn_blocking(ensure_chroma_runtime_sync)
        .await
        .map_err(|e| format!("Index runtime setup task failed: {e}"))?
}

#[tauri::command]
pub async fn index_project_cmd(project_root: String) -> Result<IndexProjectResult, String> {
    tokio::task::spawn_blocking(move || {
        let resp = run_files_action_with_crash_recovery("index", &project_root, None, None)?;
        Ok(IndexProjectResult {
            files_indexed: resp.files_indexed.unwrap_or(0),
            duration_ms: resp.duration_ms.unwrap_or(0),
        })
    })
    .await
    .map_err(|e| format!("Index project task failed: {e}"))?
}

#[tauri::command]
pub async fn search_project_index_cmd(
    project_root: String,
    query: String,
    n_results: Option<u32>,
) -> Result<Vec<SearchResultItem>, String> {
    tokio::task::spawn_blocking(move || {
        let resp =
            run_files_action_with_crash_recovery("search", &project_root, Some(&query), n_results)?;
        Ok(resp.results.unwrap_or_default())
    })
    .await
    .map_err(|e| format!("Search project index task failed: {e}"))?
}

#[tauri::command]
pub async fn get_index_status_cmd(project_root: String) -> Result<IndexStatusResult, String> {
    tokio::task::spawn_blocking(move || {
        let resp = run_files_action_with_crash_recovery("status", &project_root, None, None)?;
        Ok(IndexStatusResult {
            indexed: resp.indexed.unwrap_or(false),
            total_files: resp.total_files.unwrap_or(0),
            db_size_bytes: resp.db_size_bytes.unwrap_or(0),
        })
    })
    .await
    .map_err(|e| format!("Get index status task failed: {e}"))?
}

#[tauri::command]
pub async fn index_github_issues_cmd(project_root: String) -> Result<IndexIssuesResult, String> {
    tokio::task::spawn_blocking(move || {
        let resp = run_project_index_action_with_crash_recovery(
            "index-issues",
            &project_root,
            None,
            None,
            None,
        )?;
        Ok(IndexIssuesResult {
            issues_indexed: resp.issues_indexed.unwrap_or(0),
            duration_ms: resp.duration_ms.unwrap_or(0),
        })
    })
    .await
    .map_err(|e| format!("Index GitHub issues task failed: {e}"))?
}

#[tauri::command]
pub async fn search_github_issues_cmd(
    project_root: String,
    query: String,
    n_results: Option<u32>,
) -> Result<Vec<GitHubIssueSearchResult>, String> {
    tokio::task::spawn_blocking(move || {
        let resp = run_project_index_action_with_crash_recovery(
            "search-issues",
            &project_root,
            None,
            Some(&query),
            n_results,
        )?;
        Ok(resp.issue_results.unwrap_or_default())
    })
    .await
    .map_err(|e| format!("Search GitHub issues task failed: {e}"))?
}

/// Build index for a project in the background. Non-fatal on errors.
pub fn spawn_background_index(project_root: String) {
    tauri::async_runtime::spawn_blocking(move || {
        // 1. Ensure runtime
        if let Err(e) = ensure_chroma_runtime_sync() {
            warn!(
                category = "project_index",
                error = %e,
                "Failed to ensure chroma runtime for project index"
            );
            return;
        }

        // 2. Build index
        match run_files_action_with_crash_recovery("index", &project_root, None, None) {
            Ok(resp) => {
                tracing::info!(
                    category = "project_index",
                    files_indexed = resp.files_indexed.unwrap_or(0),
                    duration_ms = resp.duration_ms.unwrap_or(0),
                    "Project index built successfully"
                );
            }
            Err(e) => {
                warn!(
                    category = "project_index",
                    error = %e,
                    "Failed to build project index"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn managed_chroma_python() -> Option<PathBuf> {
        let venv_dir = chroma_venv_dir().ok()?;
        let python = chroma_python_path(&venv_dir);
        python.is_file().then_some(python)
    }

    fn write_project_file(path: &Path, contents: &str) {
        let parent = path.parent().expect("project file parent");
        fs::create_dir_all(parent).expect("create project file parent");
        fs::write(path, contents).expect("write project file");
    }

    fn copy_dir_recursively(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).expect("create destination dir");
        for entry in fs::read_dir(src).expect("read source dir") {
            let entry = entry.expect("read dir entry");
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if entry.file_type().expect("read entry type").is_dir() {
                copy_dir_recursively(&src_path, &dst_path);
            } else {
                fs::copy(&src_path, &dst_path).expect("copy file");
            }
        }
    }

    fn init_repo_with_worktree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempdir().expect("create tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo dir");

        let output = command_os("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("run git init");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        for (key, value) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ] {
            let output = command_os("git")
                .current_dir(&repo)
                .args(["config", key, value])
                .output()
                .expect("run git config");
            assert!(
                output.status.success(),
                "git config {key} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fs::write(repo.join("README.md"), "# Repo\n").expect("write README");

        let output = command_os("git")
            .current_dir(&repo)
            .args(["add", "README.md"])
            .output()
            .expect("run git add");
        assert!(
            output.status.success(),
            "git add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let output = command_os("git")
            .current_dir(&repo)
            .args(["commit", "-m", "initial commit"])
            .output()
            .expect("run git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let worktree = temp.path().join(".worktrees").join("feature");
        let output = command_os("git")
            .current_dir(&repo)
            .args(["worktree", "add", "-b", "feature"])
            .arg(&worktree)
            .output()
            .expect("run git worktree add");
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        (temp, repo, worktree)
    }

    #[test]
    fn db_path_uses_gwt_index_dir() {
        let path = db_path_for_project("/tmp/myproject");
        assert_eq!(path, PathBuf::from("/tmp/myproject/.gwt/index"));
    }

    #[test]
    fn normalize_project_root_for_index_moves_legacy_worktree_index() {
        let (_temp, repo, worktree) = init_repo_with_worktree();
        let legacy_db = db_path_for_project(&worktree);
        fs::create_dir_all(&legacy_db).expect("create legacy index dir");
        fs::write(legacy_db.join("chroma.sqlite3"), b"legacy").expect("write legacy db");

        let normalized = normalize_project_root_for_index(&worktree.to_string_lossy())
            .expect("normalize worktree project root");

        assert_eq!(normalized, repo);
        assert!(
            db_path_for_project(&repo).join("chroma.sqlite3").is_file(),
            "normalized project root should receive the migrated index db"
        );
        assert!(
            !legacy_db.exists(),
            "legacy worktree index directory should be moved away"
        );
        assert!(
            !worktree.join(".gwt").exists(),
            "empty legacy .gwt directory should be removed"
        );
    }

    #[test]
    fn normalize_project_root_for_index_keeps_existing_main_repo_index() {
        let (_temp, repo, worktree) = init_repo_with_worktree();
        let main_db = db_path_for_project(&repo);
        fs::create_dir_all(&main_db).expect("create normalized index dir");
        fs::write(main_db.join("chroma.sqlite3"), b"main").expect("write normalized db");

        let legacy_db = db_path_for_project(&worktree);
        fs::create_dir_all(&legacy_db).expect("create legacy index dir");
        fs::write(legacy_db.join("chroma.sqlite3"), b"legacy").expect("write legacy db");

        let normalized = normalize_project_root_for_index(&worktree.to_string_lossy())
            .expect("normalize worktree project root");

        assert_eq!(normalized, repo);
        assert_eq!(
            fs::read(main_db.join("chroma.sqlite3")).expect("read normalized db"),
            b"main"
        );
        assert_eq!(
            fs::read(legacy_db.join("chroma.sqlite3")).expect("read legacy db"),
            b"legacy"
        );
    }

    #[test]
    fn chroma_venv_dir_is_under_runtime() {
        let dir = chroma_venv_dir().unwrap();
        assert!(dir.to_string_lossy().contains("chroma-venv"));
    }

    #[test]
    fn system_python_candidates_include_windows_launcher_only_on_windows() {
        let candidates = system_python_candidates();

        #[cfg(windows)]
        {
            let py_index = candidates
                .iter()
                .position(|candidate| *candidate == "py")
                .expect("Windows candidates must include py launcher");
            let python_index = candidates
                .iter()
                .position(|candidate| *candidate == "python")
                .expect("Windows candidates must include python executable");
            assert!(
                py_index < python_index,
                "py launcher must be tried before bare python to avoid WindowsApps stubs"
            );
        }

        #[cfg(not(windows))]
        {
            assert!(
                !candidates.contains(&"py"),
                "non-Windows candidates must not include py launcher"
            );
        }
    }

    #[test]
    fn windows_store_python_alias_detection_matches_known_alias_paths() {
        #[cfg(windows)]
        {
            assert!(is_windows_store_python_alias(Path::new(
                r"C:\Users\example\AppData\Local\Microsoft\WindowsApps\python.exe"
            )));
            assert!(is_windows_store_python_alias(Path::new(
                r"C:\Users\example\AppData\Local\Microsoft\WindowsApps\python3.exe"
            )));
            assert!(is_windows_store_python_alias(Path::new(
                r"C:\Users\example\AppData\Local\Microsoft\WindowsApps\python3.13.exe"
            )));
        }

        #[cfg(not(windows))]
        {
            assert!(!is_windows_store_python_alias(Path::new(
                r"C:\Users\example\AppData\Local\Microsoft\WindowsApps\python.exe"
            )));
            assert!(!is_windows_store_python_alias(Path::new(
                r"C:\Users\example\AppData\Local\Microsoft\WindowsApps\python3.exe"
            )));
            assert!(!is_windows_store_python_alias(Path::new(
                r"C:\Users\example\AppData\Local\Microsoft\WindowsApps\python3.13.exe"
            )));
        }

        assert!(!is_windows_store_python_alias(Path::new(
            r"C:\Python313\python.exe"
        )));
        assert!(!is_windows_store_python_alias(Path::new(
            r"C:\Windows\py.exe"
        )));
    }

    #[test]
    fn chroma_collection_names_are_files_and_issues() {
        assert!(
            CHROMA_HELPER_SCRIPT.contains("\"files\""),
            "files collection name must be 'files'"
        );
        assert!(
            CHROMA_HELPER_SCRIPT.contains("\"issues\""),
            "issues collection name must be 'issues'"
        );
        assert!(
            !CHROMA_HELPER_SCRIPT.contains("\"project_files\""),
            "old collection name 'project_files' must not exist"
        );
        assert!(
            !CHROMA_HELPER_SCRIPT.contains("\"github_issues\""),
            "old collection name 'github_issues' must not exist"
        );
    }

    #[test]
    fn chroma_helper_script_emits_ascii_safe_json() {
        assert!(
            CHROMA_HELPER_SCRIPT.contains("ensure_ascii=True"),
            "chroma helper must serialize stdout JSON with ensure_ascii=True"
        );
        assert!(
            !CHROMA_HELPER_SCRIPT.contains("ensure_ascii=False"),
            "chroma helper must not emit locale-dependent non-ASCII JSON bytes"
        );
    }

    #[test]
    fn chroma_search_has_substring_fallback_for_empty_semantic_results() {
        assert!(
            CHROMA_HELPER_SCRIPT.contains("def fallback_substring_search("),
            "fallback_substring_search helper must exist"
        );
        assert!(
            CHROMA_HELPER_SCRIPT.contains("if not items:"),
            "search path must branch on empty semantic results"
        );
        assert!(
            CHROMA_HELPER_SCRIPT
                .contains("items = fallback_substring_search(collection, query, actual_n)"),
            "search path must invoke fallback_substring_search"
        );
    }

    #[test]
    fn chroma_runner_response_deserializes_camel_case_metrics() {
        let json = r#"{
            "ok": true,
            "filesIndexed": 42,
            "durationMs": 1234,
            "indexed": true,
            "totalFiles": 42,
            "dbSizeBytes": 2048
        }"#;

        let parsed: ChromaRunnerResponse =
            serde_json::from_str(json).expect("parse runner response");
        assert_eq!(parsed.files_indexed, Some(42));
        assert_eq!(parsed.duration_ms, Some(1234));
        assert_eq!(parsed.indexed, Some(true));
        assert_eq!(parsed.total_files, Some(42));
        assert_eq!(parsed.db_size_bytes, Some(2048));
    }

    #[test]
    fn repo_path_for_issue_index_resolves_bare_repo_from_project_config() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path();

        fs::create_dir_all(root.join(".gwt")).expect("create .gwt");
        fs::write(
            root.join(".gwt/project.json"),
            r#"{"bare_repo_name":"repo.git","migrated_at":"2026-01-01T00:00:00Z"}"#,
        )
        .expect("write project.json");

        let bare = root.join("repo.git");
        let output = command_os("git")
            .arg("init")
            .arg("--bare")
            .arg(&bare)
            .output()
            .expect("run git init --bare");
        assert!(
            output.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let resolved =
            repo_path_for_issue_index(&root.to_string_lossy()).expect("resolve issue index repo");
        assert_eq!(resolved, bare);
    }

    #[test]
    fn crash_recovery_detector_matches_sigsegv_signatures() {
        assert!(is_recoverable_chroma_db_error(
            "Chroma helper failed (status=signal: 11, signal=11): <no stderr>"
        ));
        assert!(is_recoverable_chroma_db_error(
            "Fatal Python error: Segmentation fault"
        ));
        assert!(!is_recoverable_chroma_db_error(
            "Chroma helper failed (status=exit status: 1): Missing Python package: chromadb"
        ));
    }

    #[test]
    fn quarantine_index_db_renames_directory_to_crashed_sibling() {
        let temp = tempdir().expect("create tempdir");
        let db = temp.path().join("index");
        fs::create_dir_all(&db).expect("create fake index dir");
        fs::write(db.join("chroma.sqlite3"), b"stub").expect("write fake sqlite file");

        let backup = quarantine_index_db(&db).expect("quarantine fake index dir");

        assert!(!db.exists(), "original db path should be moved away");
        assert!(backup.exists(), "backup path should exist after quarantine");
        assert!(
            backup
                .file_name()
                .expect("backup file name")
                .to_string_lossy()
                .starts_with("index.crashed-"),
            "backup path should use the crash quarantine naming convention"
        );
    }

    #[test]
    #[ignore = "manual recovery harness using a copied persisted db from GWT_CHROMA_REPRO_SOURCE_DB"]
    fn manual_crash_recovery_recovers_copied_persisted_index_from_env() {
        let Some(source_db) = std::env::var_os("GWT_CHROMA_REPRO_SOURCE_DB") else {
            eprintln!("skipping manual recovery test: GWT_CHROMA_REPRO_SOURCE_DB not set");
            return;
        };
        let source_db = PathBuf::from(source_db);
        if !source_db.is_dir() {
            eprintln!(
                "skipping manual recovery test: source db missing at {}",
                source_db.display()
            );
            return;
        }

        let temp = tempdir().expect("create temp project root");
        let project_root = temp.path();
        write_project_file(
            &project_root.join("README.md"),
            "# Recovery Harness\nThis project is rebuilt after quarantining a copied index.\n",
        );

        let copied_db = project_root.join(".gwt").join("index");
        copy_dir_recursively(&source_db, &copied_db);

        let response = run_files_action_with_crash_recovery(
            "status",
            &project_root.to_string_lossy(),
            None,
            None,
        )
        .expect("recovery should rebuild copied crashing index");

        assert!(copied_db.is_dir(), "rebuilt index dir should exist");
        assert!(
            fs::read_dir(project_root.join(".gwt"))
                .expect("read .gwt dir")
                .filter_map(Result::ok)
                .any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("index.crashed-")
                }),
            "quarantined backup should be left next to the rebuilt index"
        );
        assert!(
            response.indexed.unwrap_or(false),
            "status after recovery should report the rebuilt index as present"
        );
    }

    #[test]
    fn issues_rebuild_decision_rebuilds_when_probe_fails() {
        assert!(should_rebuild_issues_collection(
            "status",
            Err("sqlite metadata probe failed".to_string())
        ));
    }

    #[test]
    fn issues_rebuild_decision_skips_when_probe_confirms_absent_for_files_actions() {
        assert!(!should_rebuild_issues_collection("status", Ok(false)));
    }

    #[test]
    fn issues_rebuild_decision_always_rebuilds_for_issue_actions() {
        assert!(should_rebuild_issues_collection("search-issues", Ok(false)));
    }

    #[test]
    fn managed_chroma_runtime_probe_succeeds_when_available() {
        let Some(python) = managed_chroma_python() else {
            eprintln!("skipping managed Chroma runtime probe test: runtime not present");
            return;
        };

        let response = run_chroma_runner(&python, "probe", None, None, None, None)
            .expect("managed Chroma runtime probe should succeed");

        assert!(response.ok, "probe response must be ok");
        assert!(
            response.error.is_none(),
            "probe response must not include an error"
        );
    }

    #[test]
    fn managed_chroma_runtime_can_index_and_search_temp_project() {
        let Some(python) = managed_chroma_python() else {
            eprintln!("skipping managed Chroma runtime integration test: runtime not present");
            return;
        };

        let project = tempdir().expect("create project tempdir");
        write_project_file(
            &project.path().join("src/lib.rs"),
            "//! alpha project index smoke test\npub fn alpha_probe() {}\n",
        );
        write_project_file(
            &project.path().join("README.md"),
            "# Alpha Project\nThis repository exists for chroma integration tests.\n",
        );

        let db = tempdir().expect("create db tempdir");
        let project_root = project.path().to_string_lossy().to_string();
        let db_path = db.path().to_string_lossy().to_string();

        let index_response = run_chroma_runner(
            &python,
            "index",
            Some(&project_root),
            Some(&db_path),
            None,
            None,
        )
        .expect("managed Chroma runtime index should succeed");

        assert_eq!(
            index_response.files_indexed,
            Some(2),
            "expected both fixture files to be indexed"
        );

        let search_response = run_chroma_runner(
            &python,
            "search",
            None,
            Some(&db_path),
            Some("alpha_probe"),
            Some(5),
        )
        .expect("managed Chroma runtime search should succeed");

        let results = search_response.results.expect("search results");
        assert!(
            results.iter().any(|item| item.path == "src/lib.rs"),
            "search results must include src/lib.rs: {:?}",
            results
                .iter()
                .map(|item| item.path.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[ignore = "manual reproduction harness for issue #1519 using the local Chroma runtime"]
    fn managed_chroma_runtime_probe_is_stable_across_repeated_process_runs() {
        let python = managed_chroma_python().expect("managed Chroma runtime must be present");

        for attempt in 1..=10 {
            run_chroma_runner(&python, "probe", None, None, None, None).unwrap_or_else(|err| {
                panic!("managed Chroma runtime probe failed on attempt {attempt}: {err}")
            });
        }
    }

    #[test]
    #[ignore = "manual parallel reproduction harness for issue #1519 using the local Chroma runtime"]
    fn managed_chroma_runtime_probe_is_stable_across_parallel_process_runs() {
        let python = managed_chroma_python().expect("managed Chroma runtime must be present");
        let mut workers = Vec::new();

        for worker_id in 0..4 {
            let python = python.clone();
            workers.push(std::thread::spawn(move || {
                for attempt in 1..=5 {
                    run_chroma_runner(&python, "probe", None, None, None, None).unwrap_or_else(
                        |err| {
                            panic!(
                                "parallel managed Chroma runtime probe failed on worker {worker_id} attempt {attempt}: {err}"
                            )
                        },
                    );
                }
            }));
        }

        for worker in workers {
            worker
                .join()
                .expect("parallel probe worker must join cleanly");
        }
    }
}
