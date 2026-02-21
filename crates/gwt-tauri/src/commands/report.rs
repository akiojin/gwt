//! Tauri commands for error reporting and feature suggestions.

use gwt_core::StructuredError;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSystemInfo {
    pub os_name: String,
    pub os_version: String,
    pub arch: String,
    pub gwt_version: String,
}

fn log_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "gwt")
        .map(|p| p.data_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from(".gwt/logs"))
}

/// Read the last `max_lines` lines from the most recent log file.
#[tauri::command]
pub fn read_recent_logs(max_lines: Option<u32>) -> Result<String, StructuredError> {
    let max = max_lines.unwrap_or(100) as usize;
    let log_base = log_dir();

    // Find the most recent .jsonl file across workspace dirs
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(&log_base) {
        for entry in entries.flatten() {
            let ws_dir = entry.path();
            if ws_dir.is_dir() {
                if let Ok(files) = fs::read_dir(&ws_dir) {
                    for f in files.flatten() {
                        let p = f.path();
                        if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                            candidates.push(p);
                        }
                    }
                }
            }
        }
    }

    if candidates.is_empty() {
        return Ok("(No log files found)".to_string());
    }

    // Sort by modification time (most recent first)
    candidates.sort_by(|a, b| {
        let ma = a.metadata().and_then(|m| m.modified()).ok();
        let mb = b.metadata().and_then(|m| m.modified()).ok();
        mb.cmp(&ma)
    });

    let content = fs::read_to_string(&candidates[0])
        .map_err(|e| StructuredError::internal(&e.to_string(), "read_recent_logs"))?;

    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max);
    Ok(lines[start..].join("\n"))
}

/// Get basic system info for error reports.
#[tauri::command]
pub fn get_report_system_info() -> ReportSystemInfo {
    ReportSystemInfo {
        os_name: std::env::consts::OS.to_string(),
        os_version: os_version(),
        arch: std::env::consts::ARCH.to_string(),
        gwt_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        gwt_core::process::command("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(target_os = "windows")]
    {
        gwt_core::process::command("cmd")
            .args(["/c", "ver"])
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        gwt_core::process::command("uname")
            .arg("-r")
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    }
}
