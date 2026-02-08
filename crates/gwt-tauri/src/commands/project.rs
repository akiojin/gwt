//! Project/repo management commands

use crate::state::AppState;
use gwt_core::git::{self, Branch};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{fs, io::Read};
use tauri::State;
use tauri::{AppHandle, Emitter};

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
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CloneProgress {
    pub stage: String,
    pub percent: u8,
}

/// Open a project (set project_path in AppState)
#[tauri::command]
pub fn open_project(path: String, state: State<AppState>) -> Result<ProjectInfo, String> {
    let p = Path::new(&path);

    if !p.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    if !git::is_git_repo(p) {
        return Err(format!("Not a git repository: {}", path));
    }

    // Get repo name from the directory name
    let repo_name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());

    // Get current branch
    let current_branch = Branch::current(p).ok().flatten().map(|b| b.name);

    // Update state
    if let Ok(mut project_path) = state.project_path.lock() {
        *project_path = Some(path.clone());
    }

    Ok(ProjectInfo {
        path,
        repo_name,
        current_branch,
    })
}

/// Get current project info from state
#[tauri::command]
pub fn get_project_info(state: State<AppState>) -> Option<ProjectInfo> {
    let project_path = state.project_path.lock().ok()?;
    let path_str = project_path.as_ref()?;
    let p = Path::new(path_str);

    let repo_name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path_str.clone());

    let current_branch = Branch::current(p).ok().flatten().map(|b| b.name);

    Some(ProjectInfo {
        path: path_str.clone(),
        repo_name,
        current_branch,
    })
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
/// and then opening it (updating `AppState.project_path`).
#[tauri::command]
pub fn create_project(
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
    let mut child = Command::new("git")
        .args([
            "clone",
            "--bare",
            "--progress",
            &request.repo_url,
            &target.to_string_lossy(),
        ])
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

    // Open the cloned repository (FR-304)
    open_project(target.to_string_lossy().to_string(), state)
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
}
