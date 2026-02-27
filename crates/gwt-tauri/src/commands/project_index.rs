//! Project structure index commands backed by ChromaDB (Python runtime).

use gwt_core::process::command_os;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::warn;

const CHROMA_VENV_DIR: &str = "chroma-venv";
const CHROMA_RUNTIME_PIP_DEPS: &[&str] = &["chromadb"];
const CHROMA_HELPER_SCRIPT: &str = include_str!("../python/chroma_index_runner.py");

static CHROMA_RUNTIME_PROBE: Mutex<Option<Result<(), String>>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ChromaRunnerResponse {
    ok: bool,
    error: Option<String>,
    #[serde(default)]
    files_indexed: Option<u64>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    results: Option<Vec<SearchResultItem>>,
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
    #[cfg(windows)]
    {
        return venv_dir.join("Scripts").join("python.exe");
    }

    #[cfg(not(windows))]
    {
        venv_dir.join("bin").join("python3")
    }
}

fn find_system_python() -> Result<PathBuf, String> {
    for candidate in ["python3.12", "python3.11", "python3", "python"] {
        if let Ok(path) = which::which(candidate) {
            return Ok(path);
        }
    }
    Err("Python runtime not found (checked python3.12/python3.11/python3/python)".to_string())
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
            let _ = fs::set_permissions(&script_path, perm);
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
        return Err(format!(
            "Chroma helper failed (status={}): {}{}",
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
    if probe_chroma_runtime_cached().is_ok() {
        let python = find_python_binary()?;
        return Ok(IndexRuntimeSetupResult {
            ready: true,
            installed: false,
            python_path: python.to_string_lossy().to_string(),
        });
    }

    let runtime_dir = gwt_runtime_dir()?;
    fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("Failed to create runtime directory: {e}"))?;

    let venv_dir = chroma_venv_dir()?;
    let managed_python = chroma_python_path(&venv_dir);
    let mut installed = false;

    if !managed_python.is_file() {
        let bootstrap_python = find_system_python()?;
        let mut cmd = command_os(&bootstrap_python);
        cmd.arg("-m").arg("venv").arg(&venv_dir);
        run_command_with_output(cmd, "Failed to create chroma runtime virtual environment")?;
        installed = true;
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

fn db_path_for_project(project_root: &str) -> PathBuf {
    Path::new(project_root).join(".gwt").join("index")
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
        let python = find_python_binary()?;
        let db = db_path_for_project(&project_root);
        let resp = run_chroma_runner(
            &python,
            "index",
            Some(&project_root),
            Some(&db.to_string_lossy()),
            None,
            None,
        )?;
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
        let python = find_python_binary()?;
        let db = db_path_for_project(&project_root);
        let resp = run_chroma_runner(
            &python,
            "search",
            None,
            Some(&db.to_string_lossy()),
            Some(&query),
            n_results,
        )?;
        Ok(resp.results.unwrap_or_default())
    })
    .await
    .map_err(|e| format!("Search project index task failed: {e}"))?
}

#[tauri::command]
pub async fn get_index_status_cmd(project_root: String) -> Result<IndexStatusResult, String> {
    tokio::task::spawn_blocking(move || {
        let python = find_python_binary()?;
        let db = db_path_for_project(&project_root);
        let resp = run_chroma_runner(
            &python,
            "status",
            None,
            Some(&db.to_string_lossy()),
            None,
            None,
        )?;
        Ok(IndexStatusResult {
            indexed: resp.indexed.unwrap_or(false),
            total_files: resp.total_files.unwrap_or(0),
            db_size_bytes: resp.db_size_bytes.unwrap_or(0),
        })
    })
    .await
    .map_err(|e| format!("Get index status task failed: {e}"))?
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
        let python = match find_python_binary() {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    category = "project_index",
                    error = %e,
                    "Failed to find python for project index"
                );
                return;
            }
        };

        let db = db_path_for_project(&project_root);
        match run_chroma_runner(
            &python,
            "index",
            Some(&project_root),
            Some(&db.to_string_lossy()),
            None,
            None,
        ) {
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

    #[test]
    fn db_path_uses_gwt_index_dir() {
        let path = db_path_for_project("/tmp/myproject");
        assert_eq!(path, PathBuf::from("/tmp/myproject/.gwt/index"));
    }

    #[test]
    fn chroma_venv_dir_is_under_runtime() {
        let dir = chroma_venv_dir().unwrap();
        assert!(dir.to_string_lossy().contains("chroma-venv"));
    }
}
