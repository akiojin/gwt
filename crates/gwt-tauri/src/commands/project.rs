//! Project/repo management commands

use crate::state::AppState;
use gwt_core::git::{self, Branch};
use gwt_core::migration::{
    derive_bare_repo_name, execute_migration, rollback_migration, MigrationConfig, MigrationState,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{fs, io::Read};
use tauri::Manager;
use tauri::State;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

/// Serializable project info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct ProjectInfo {
    pub path: String,
    pub repo_name: String,
    pub current_branch: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProjectRequest {
    pub repo_url: String,
    pub parent_dir: String,
    #[serde(default)]
    pub shallow: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CloneProgress {
    pub stage: String,
    pub percent: u8,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectJsonConfig {
    pub bare_repo_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectTomlConfig {
    pub bare_repo_name: String,
}

fn read_bare_repo_name(project_root: &Path) -> Option<String> {
    let gwt_dir = project_root.join(".gwt");
    if !gwt_dir.is_dir() {
        return None;
    }

    let toml_path = gwt_dir.join("project.toml");
    if let Ok(content) = std::fs::read_to_string(&toml_path) {
        if let Ok(cfg) = toml::from_str::<ProjectTomlConfig>(&content) {
            if !cfg.bare_repo_name.trim().is_empty() {
                return Some(cfg.bare_repo_name);
            }
        }
    }

    let json_path = gwt_dir.join("project.json");
    if let Ok(content) = std::fs::read_to_string(&json_path) {
        if let Ok(cfg) = serde_json::from_str::<ProjectJsonConfig>(&content) {
            if !cfg.bare_repo_name.trim().is_empty() {
                return Some(cfg.bare_repo_name);
            }
        }
    }

    None
}

fn resolve_project_root(selected: &Path) -> std::path::PathBuf {
    if git::is_git_repo(selected) {
        if git::is_bare_repository(selected) {
            selected.parent().unwrap_or(selected).to_path_buf()
        } else {
            // If selected is a worktree, this resolves to the bare project's root directory.
            git::get_main_repo_root(selected)
        }
    } else {
        selected.to_path_buf()
    }
}

pub(crate) fn resolve_repo_path_for_project_root(
    project_root: &Path,
) -> Result<std::path::PathBuf, String> {
    if git::is_git_repo(project_root) {
        return Ok(project_root.to_path_buf());
    }

    if let Some(bare_repo_name) = read_bare_repo_name(project_root) {
        let candidate = project_root.join(&bare_repo_name);
        if candidate.exists() && git::is_bare_repository(&candidate) {
            return Ok(candidate);
        }
    }

    // Fallback: try to detect a bare repo under the selected directory.
    if let Some(bare) = git::find_bare_repo_in_dir(project_root) {
        return Ok(bare);
    }

    Err(format!(
        "Not a gwt project: bare repository not found in {}",
        project_root.display()
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbePathResult {
    /// "gwtProject" | "migrationRequired" | "emptyDir" | "notFound" | "invalid" | "notGwtProject"
    pub kind: String,
    /// Canonical project root path for opening (when kind == "gwtProject")
    pub project_path: Option<String>,
    /// Normal repository root that must be migrated (when kind == "migrationRequired")
    pub migration_source_root: Option<String>,
    /// Best-effort detail for UI error messages
    pub message: Option<String>,
}

fn dir_is_empty(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut it) => it.next().is_none(),
        Err(_) => false,
    }
}

/// Probe a path and return how the GUI should handle it.
#[tauri::command]
pub fn probe_path(path: String) -> ProbePathResult {
    let p = Path::new(&path);

    if !p.exists() {
        return ProbePathResult {
            kind: "notFound".to_string(),
            project_path: None,
            migration_source_root: None,
            message: Some(format!("Path does not exist: {}", path)),
        };
    }

    if !p.is_dir() {
        return ProbePathResult {
            kind: "invalid".to_string(),
            project_path: None,
            migration_source_root: None,
            message: Some("Path is not a directory".to_string()),
        };
    }

    let project_root = resolve_project_root(p);

    match resolve_repo_path_for_project_root(&project_root) {
        Ok(repo_path) => {
            if git::is_bare_repository(&repo_path) {
                return ProbePathResult {
                    kind: "gwtProject".to_string(),
                    project_path: Some(project_root.to_string_lossy().to_string()),
                    migration_source_root: None,
                    message: None,
                };
            }

            // Not a bare repo: enforce migration (SPEC-a70a1ece US7).
            let source_root = git::get_main_repo_root(&repo_path);
            ProbePathResult {
                kind: "migrationRequired".to_string(),
                project_path: None,
                migration_source_root: Some(source_root.to_string_lossy().to_string()),
                message: Some("Migration to a bare gwt project is required.".to_string()),
            }
        }
        Err(err) => {
            if project_root.is_dir() && dir_is_empty(&project_root) {
                return ProbePathResult {
                    kind: "emptyDir".to_string(),
                    project_path: Some(project_root.to_string_lossy().to_string()),
                    migration_source_root: None,
                    message: None,
                };
            }

            ProbePathResult {
                kind: "notGwtProject".to_string(),
                project_path: None,
                migration_source_root: None,
                message: Some(err),
            }
        }
    }
}

/// Open a project (set project_path in AppState)
#[tauri::command]
pub fn open_project(
    window: tauri::Window,
    path: String,
    state: State<AppState>,
) -> Result<ProjectInfo, String> {
    let p = Path::new(&path);

    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    let project_root = resolve_project_root(p);
    let repo_path = resolve_repo_path_for_project_root(&project_root)?;
    if git::is_git_repo(&repo_path) && !git::is_bare_repository(&repo_path) {
        return Err("Migration required: normal repositories are not supported. Please migrate to a bare gwt project.".to_string());
    }
    let project_root_str = project_root.to_string_lossy().to_string();

    // Get repo name from the directory name
    let repo_name = project_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project_root_str.clone());

    // Get current branch
    let current_branch = Branch::current(&repo_path).ok().flatten().map(|b| b.name);

    // Update window-scoped state
    state.set_project_for_window(window.label(), project_root_str.clone());

    // Record to recent projects history
    let _ = gwt_core::config::record_recent_project(&project_root_str);

    let _ = crate::menu::rebuild_menu(window.app_handle());

    Ok(ProjectInfo {
        path: project_root_str,
        repo_name,
        current_branch,
    })
}

/// Get current project info from state
#[tauri::command]
pub fn get_project_info(window: tauri::Window, state: State<AppState>) -> Option<ProjectInfo> {
    let path_str = state.project_for_window(window.label())?;
    let p = Path::new(&path_str);

    let repo_name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path_str.clone());

    let current_branch = resolve_repo_path_for_project_root(p)
        .ok()
        .and_then(|repo_path| Branch::current(&repo_path).ok().flatten().map(|b| b.name));

    Some(ProjectInfo {
        path: path_str,
        repo_name,
        current_branch,
    })
}

/// Close the current window's project (clear window-scoped state)
#[tauri::command]
pub fn close_project(window: tauri::Window, state: State<AppState>) -> Result<(), String> {
    state.clear_project_for_window(window.label());
    let _ = crate::menu::rebuild_menu(window.app_handle());
    Ok(())
}

/// Check if a path is a git repository
#[tauri::command]
pub fn is_git_repo(path: String) -> bool {
    git::is_git_repo(Path::new(&path))
}

fn is_valid_github_repo_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return false;
    }

    // Reject URLs with query/fragment to avoid ambiguity.
    if url.contains('?') || url.contains('#') {
        return false;
    }

    let rest = if let Some(r) = url.strip_prefix("https://github.com/") {
        r
    } else if let Some(r) = url.strip_prefix("http://github.com/") {
        r
    } else if let Some(r) = url.strip_prefix("git@github.com:") {
        r
    } else if let Some(r) = url.strip_prefix("ssh://git@github.com/") {
        r
    } else {
        return false;
    };

    let rest = rest.trim_end_matches('/');
    let mut parts = rest.split('/');
    let owner = match parts.next() {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    let repo = match parts.next() {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    if parts.next().is_some() {
        return false;
    }

    if !is_valid_github_segment(owner) {
        return false;
    }

    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if repo.is_empty() {
        return false;
    }
    is_valid_github_segment(repo)
}

fn is_valid_github_segment(seg: &str) -> bool {
    seg.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn parse_clone_progress_line(line: &str) -> Option<CloneProgress> {
    let line = line.trim();
    let stage = if line.starts_with("Receiving objects:") {
        "receiving"
    } else if line.starts_with("Resolving deltas:") {
        "resolving"
    } else {
        return None;
    };

    let percent = extract_percent(line)?;
    Some(CloneProgress {
        stage: stage.to_string(),
        percent,
    })
}

fn extract_percent(s: &str) -> Option<u8> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'%' {
                if let Ok(n) = s[start..j].parse::<u16>() {
                    if n <= 100 {
                        return Some(n as u8);
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// Create a new project by bare-cloning a GitHub repository into `<parent>/<repo>.git`
/// and then opening it (updating window-scoped project state).
#[tauri::command]
pub fn create_project(
    window: tauri::Window,
    request: NewProjectRequest,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<ProjectInfo, String> {
    if !is_valid_github_repo_url(&request.repo_url) {
        return Err("Invalid repository URL".to_string());
    }

    let parent = std::path::PathBuf::from(&request.parent_dir);
    if !parent.exists() {
        return Err(format!(
            "Parent directory does not exist: {}",
            request.parent_dir
        ));
    }

    let repo_name = git::extract_repo_name(&request.repo_url);
    let target = parent.join(&repo_name);
    if target.exists() {
        return Err(format!(
            "Target directory already exists: {}",
            target.display()
        ));
    }

    // Run `git clone --bare --progress` and stream progress via events.
    let mut args: Vec<String> = vec![
        "clone".to_string(),
        "--bare".to_string(),
        "--progress".to_string(),
    ];
    if request.shallow {
        args.push("--depth=1".to_string());
    }
    args.push(request.repo_url.clone());
    args.push(target.to_string_lossy().to_string());

    let mut child = Command::new("git")
        .args(args)
        .current_dir(&parent)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute git clone: {}", e))?;

    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Failed to capture git clone output".to_string())?;

    let mut buf = [0u8; 4096];
    let mut raw: Vec<u8> = Vec::new();
    let mut line_buf: Vec<u8> = Vec::new();
    let mut last_progress: Option<CloneProgress> = None;

    let mut flush_line = |line_buf: &mut Vec<u8>| {
        if line_buf.is_empty() {
            return;
        }
        let line = String::from_utf8_lossy(line_buf).to_string();
        line_buf.clear();

        if let Some(p) = parse_clone_progress_line(&line) {
            if last_progress.as_ref() != Some(&p) {
                let _ = app_handle.emit("clone-progress", &p);
                last_progress = Some(p);
            }
        }
    };

    loop {
        match stderr.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        flush_line(&mut line_buf);
                    } else {
                        line_buf.push(b);
                    }
                }
            }
            Err(_) => break,
        }
    }
    flush_line(&mut line_buf);

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for git clone: {}", e))?;

    if !status.success() {
        // Cleanup incomplete directory (FR-303)
        if target.exists() {
            let _ = fs::remove_dir_all(&target);
        }

        let stderr_text = String::from_utf8_lossy(&raw);
        let tail = stderr_text
            .lines()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");

        if tail.trim().is_empty() {
            return Err("git clone failed".to_string());
        }
        return Err(format!("git clone failed: {}", tail.trim()));
    }

    // Open the project root (FR-304)
    open_project(window, request.parent_dir, state)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationProgressPayload {
    pub job_id: String,
    pub state: String,
    pub current: Option<usize>,
    pub total: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationFinishedPayload {
    pub job_id: String,
    pub ok: bool,
    pub error: Option<String>,
    pub project_path: Option<String>,
}

fn encode_migration_state(state: &MigrationState) -> (String, Option<usize>, Option<usize>) {
    match state {
        MigrationState::Pending => ("pending".to_string(), None, None),
        MigrationState::Validating => ("validating".to_string(), None, None),
        MigrationState::BackingUp => ("backingUp".to_string(), None, None),
        MigrationState::CreatingBareRepo => ("creatingBareRepo".to_string(), None, None),
        MigrationState::MigratingWorktrees { current, total } => (
            "migratingWorktrees".to_string(),
            Some(current.saturating_add(1)),
            Some(*total),
        ),
        MigrationState::CleaningUp => ("cleaningUp".to_string(), None, None),
        MigrationState::Completed => ("completed".to_string(), None, None),
        MigrationState::RollingBack => ("rollingBack".to_string(), None, None),
        MigrationState::Cancelled => ("cancelled".to_string(), None, None),
        MigrationState::Failed => ("failed".to_string(), None, None),
    }
}

/// Start a bare migration job for a normal repository (SPEC-a70a1ece US7).
#[tauri::command]
pub fn start_migration_job(path: String, app_handle: AppHandle) -> Result<String, String> {
    let selected = Path::new(&path);
    if !selected.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    let source_root = git::get_main_repo_root(selected);
    if !git::is_git_repo(&source_root) {
        return Err("Not a git repository".to_string());
    }
    if git::is_bare_repository(&source_root) {
        return Err("Repository is already bare".to_string());
    }

    let job_id = Uuid::new_v4().to_string();
    let job_id_thread = job_id.clone();
    let app = app_handle.clone();
    let source_root_thread = source_root.clone();

    std::thread::spawn(move || {
        let bare_repo_name = derive_bare_repo_name(&source_root_thread.to_string_lossy());
        let config = MigrationConfig::new(
            source_root_thread.clone(),
            source_root_thread.clone(),
            bare_repo_name,
        );

        let progress_job = job_id_thread.clone();
        let app_progress = app.clone();
        let progress_cb = Box::new(move |state: MigrationState| {
            let (s, current, total) = encode_migration_state(&state);
            let payload = MigrationProgressPayload {
                job_id: progress_job.clone(),
                state: s,
                current,
                total,
            };
            let _ = app_progress.emit("migration-progress", &payload);
        });

        let result = execute_migration(&config, Some(progress_cb));

        let (ok, error) = match result {
            Ok(()) => (true, None),
            Err(err) => {
                let err_msg = err.to_string();
                let rollback = rollback_migration(&config).map_err(|e| e.to_string());
                let msg = match rollback {
                    Ok(()) => err_msg,
                    Err(rollback_err) => format!("{err_msg}\n\nRollback failed: {rollback_err}"),
                };
                (false, Some(msg))
            }
        };

        let finished = MigrationFinishedPayload {
            job_id: job_id_thread.clone(),
            ok,
            error,
            project_path: ok.then_some(source_root_thread.to_string_lossy().to_string()),
        };
        let _ = app.emit("migration-finished", &finished);
    });

    Ok(job_id)
}

/// Quit the app (used by forced migration refusal).
#[tauri::command]
pub fn quit_app(state: State<AppState>, app_handle: AppHandle) -> Result<(), String> {
    state.request_quit();
    app_handle.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_github_repo_url_accepts_https() {
        assert!(is_valid_github_repo_url("https://github.com/owner/repo"));
        assert!(is_valid_github_repo_url(
            "https://github.com/owner/repo.git"
        ));
        assert!(is_valid_github_repo_url("https://github.com/owner/repo/"));
    }

    #[test]
    fn test_is_valid_github_repo_url_accepts_ssh() {
        assert!(is_valid_github_repo_url("git@github.com:owner/repo.git"));
        assert!(is_valid_github_repo_url(
            "ssh://git@github.com/owner/repo.git"
        ));
    }

    #[test]
    fn test_is_valid_github_repo_url_rejects_invalid() {
        assert!(!is_valid_github_repo_url("https://gitlab.com/owner/repo"));
        assert!(!is_valid_github_repo_url("https://github.com/owner"));
        assert!(!is_valid_github_repo_url("https://github.com/owner/"));
        assert!(!is_valid_github_repo_url("not a url"));
        assert!(!is_valid_github_repo_url(
            "https://github.com/owner/repo?ref=main"
        ));
    }

    #[test]
    fn test_parse_clone_progress_line_receiving() {
        let p =
            parse_clone_progress_line("Receiving objects:  42% (1234/9999), 1.23 MiB | 1.23 MiB/s")
                .expect("should parse receiving progress");
        assert_eq!(
            p,
            CloneProgress {
                stage: "receiving".to_string(),
                percent: 42
            }
        );
    }

    #[test]
    fn test_parse_clone_progress_line_resolving() {
        let p = parse_clone_progress_line("Resolving deltas:  7% (1/14)")
            .expect("should parse resolving progress");
        assert_eq!(
            p,
            CloneProgress {
                stage: "resolving".to_string(),
                percent: 7
            }
        );
    }

    #[test]
    fn test_parse_clone_progress_line_ignores_other_lines() {
        assert_eq!(
            parse_clone_progress_line("remote: Counting objects: 100% (12/12)"),
            None
        );
        assert_eq!(
            parse_clone_progress_line("Cloning into 'repo.git'..."),
            None
        );
    }

    #[test]
    fn test_resolve_repo_path_for_project_root_from_project_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();

        std::fs::create_dir_all(root.join(".gwt")).expect("create .gwt dir");
        std::fs::write(
            root.join(".gwt").join("project.json"),
            r#"{"bare_repo_name":"repo.git","migrated_at":"2026-01-01T00:00:00Z"}"#,
        )
        .expect("write project.json");

        let bare = root.join("repo.git");
        let status = Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare)
            .status()
            .expect("git init --bare");
        assert!(status.success());

        let resolved =
            resolve_repo_path_for_project_root(root).expect("should resolve bare repo path");
        assert_eq!(resolved, bare);
    }

    #[test]
    fn test_resolve_repo_path_for_project_root_fallback_scans_for_bare_repo() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();

        let bare = root.join("repo.git");
        let status = Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare)
            .status()
            .expect("git init --bare");
        assert!(status.success());

        let resolved =
            resolve_repo_path_for_project_root(root).expect("should resolve bare repo path");
        assert_eq!(resolved, bare);
    }
}
