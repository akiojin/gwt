//! `gwtd diagnostics ...` family module.

use std::{
    cmp::Reverse,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use gwt_github::{client::ApiError, SpecOpsError};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{CliEnv, CliParseError};

const CPU_DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;
const RECENT_LOG_FILE_LIMIT: usize = 16;
const RECENT_LOG_LINES_PER_FILE: usize = 2_000;

/// SPEC-1939 Phase 67 family enum for `gwtd diagnostics ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticsCommand {
    /// `gwtd diagnostics cpu --json`.
    Cpu { json: bool },
}

pub fn parse(args: &[String]) -> Result<DiagnosticsCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "cpu" if rest == ["--json"] => Ok(DiagnosticsCommand::Cpu { json: true }),
        "cpu" => Err(CliParseError::Usage),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

pub fn run<E: CliEnv>(
    env: &mut E,
    cmd: DiagnosticsCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match cmd {
        DiagnosticsCommand::Cpu { json: true } => {
            let diagnostics = collect_cpu_diagnostics(env.repo_path());
            let rendered = serde_json::to_string_pretty(&diagnostics).map_err(unexpected_error)?;
            out.push_str(&rendered);
            out.push('\n');
            Ok(0)
        }
        DiagnosticsCommand::Cpu { json: false } => Err(unexpected_error("json output is required")),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CpuDiagnostics {
    pub schema_version: u32,
    pub repo_path: String,
    pub gwt_processes: Vec<ProcessSnapshot>,
    pub runner_processes: Vec<ProcessSnapshot>,
    pub binaries: BinaryDiagnostics,
    pub runtime: RuntimeDiagnostics,
    pub recent_logs: LogBudgetDiagnostics,
    pub stale: StaleDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub cpu_percent: Option<f64>,
    pub elapsed: Option<String>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileHash {
    pub path: String,
    pub sha256_16: String,
    pub size_bytes: u64,
    pub modified_unix_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BinaryDiagnostics {
    pub installed_gwt: Option<FileHash>,
    pub installed_gwtd: Option<FileHash>,
    pub current_gwt: Option<FileHash>,
    pub current_gwtd: Option<FileHash>,
    pub current_exe: Option<FileHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeDiagnostics {
    pub runtime_dir: String,
    pub manifest_path: String,
    pub manifest_runner_path: Option<String>,
    pub manifest_runner_hash: Option<String>,
    pub legacy_runner_path: String,
    pub legacy_runner_hash: Option<String>,
    pub bundled_runner_hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LogBudgetDiagnostics {
    pub inspected_files: usize,
    pub inspected_lines: usize,
    pub status_completed_info: usize,
    pub hook_live_204_info: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaleDiagnostics {
    pub installed_gwt_differs_from_current: Option<bool>,
    pub installed_gwtd_differs_from_current: Option<bool>,
    pub runtime_manifest_stale: Option<bool>,
    pub legacy_runner_stale: Option<bool>,
    pub status_or_hook_log_storm: bool,
}

pub fn collect_cpu_diagnostics(repo_path: &Path) -> CpuDiagnostics {
    let snapshots = collect_process_snapshots();
    let gwt_processes = snapshots
        .iter()
        .filter(|snapshot| is_gwt_process(snapshot))
        .cloned()
        .collect();
    let runner_processes = snapshots
        .iter()
        .filter(|snapshot| is_chroma_runner_process(snapshot))
        .cloned()
        .collect();
    let binaries = collect_binary_diagnostics();
    let runtime = collect_runtime_diagnostics();
    let recent_logs = collect_recent_log_budget(
        &gwt_core::paths::gwt_projects_dir(),
        RECENT_LOG_FILE_LIMIT,
        RECENT_LOG_LINES_PER_FILE,
    );
    let stale = StaleDiagnostics {
        installed_gwt_differs_from_current: hashes_differ(
            binaries.installed_gwt.as_ref(),
            binaries.current_gwt.as_ref(),
        ),
        installed_gwtd_differs_from_current: hashes_differ(
            binaries.installed_gwtd.as_ref(),
            binaries.current_gwtd.as_ref(),
        ),
        runtime_manifest_stale: runtime
            .manifest_runner_hash
            .as_ref()
            .map(|hash| hash != &runtime.bundled_runner_hash),
        legacy_runner_stale: runtime
            .legacy_runner_hash
            .as_ref()
            .map(|hash| hash != &runtime.bundled_runner_hash),
        status_or_hook_log_storm: recent_logs.status_completed_info > 100
            || recent_logs.hook_live_204_info > 100,
    };

    CpuDiagnostics {
        schema_version: CPU_DIAGNOSTICS_SCHEMA_VERSION,
        repo_path: repo_path.display().to_string(),
        gwt_processes,
        runner_processes,
        binaries,
        runtime,
        recent_logs,
        stale,
    }
}

fn collect_process_snapshots() -> Vec<ProcessSnapshot> {
    let output = match Command::new("ps")
        .args(["axww", "-o", "pid=,ppid=,%cpu=,etime=,command="])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_process_snapshots(&stdout)
}

fn parse_process_snapshots(text: &str) -> Vec<ProcessSnapshot> {
    text.lines()
        .filter_map(parse_process_snapshot_line)
        .collect()
}

fn parse_process_snapshot_line(line: &str) -> Option<ProcessSnapshot> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse().ok()?;
    let ppid = parts.next().and_then(|value| value.parse().ok());
    let cpu_percent = parts.next().and_then(|value| value.parse().ok());
    let elapsed = parts.next().map(ToString::to_string);
    let command = parts.collect::<Vec<_>>().join(" ");
    if command.is_empty() {
        return None;
    }
    Some(ProcessSnapshot {
        pid,
        ppid,
        cpu_percent,
        elapsed,
        command,
    })
}

fn is_gwt_process(snapshot: &ProcessSnapshot) -> bool {
    let command = snapshot.command.as_str();
    if command.contains("chroma_index_runner") || command.contains("/gwtd") {
        return false;
    }
    command.contains("/GWT.app/Contents/MacOS/gwt")
        || command.contains("/target/debug/gwt")
        || command.contains("/target/release/gwt")
}

fn is_chroma_runner_process(snapshot: &ProcessSnapshot) -> bool {
    snapshot.command.contains("chroma_index_runner")
}

fn collect_binary_diagnostics() -> BinaryDiagnostics {
    let current_exe_path = std::env::current_exe().ok();
    let current_gwtd_path = current_gwtd_candidate(current_exe_path.as_deref());
    let current_gwt_path = current_gwt_candidate(current_exe_path.as_deref());
    BinaryDiagnostics {
        installed_gwt: hash_file(Path::new("/Applications/GWT.app/Contents/MacOS/gwt")),
        installed_gwtd: hash_file(Path::new("/Applications/GWT.app/Contents/MacOS/gwtd")),
        current_gwt: current_gwt_path.as_deref().and_then(hash_file),
        current_gwtd: current_gwtd_path.as_deref().and_then(hash_file),
        current_exe: current_exe_path.as_deref().and_then(hash_file),
    }
}

fn current_gwt_candidate(current_exe: Option<&Path>) -> Option<PathBuf> {
    let current_exe = current_exe?;
    match current_exe.file_name().and_then(|name| name.to_str()) {
        Some("gwt") => Some(current_exe.to_path_buf()),
        Some("gwtd") => Some(current_exe.with_file_name("gwt")),
        _ => None,
    }
}

fn current_gwtd_candidate(current_exe: Option<&Path>) -> Option<PathBuf> {
    let current_exe = current_exe?;
    match current_exe.file_name().and_then(|name| name.to_str()) {
        Some("gwtd") => Some(current_exe.to_path_buf()),
        Some("gwt") => Some(current_exe.with_file_name("gwtd")),
        _ => None,
    }
}

fn collect_runtime_diagnostics() -> RuntimeDiagnostics {
    let runtime_dir = gwt_core::paths::gwt_runtime_dir();
    let manifest_path = gwt_core::runtime::project_index_runtime_manifest_path();
    let legacy_runner_path = gwt_core::paths::gwt_runtime_runner_path();
    let manifest = read_runtime_manifest_runner(&runtime_dir, &manifest_path);
    RuntimeDiagnostics {
        runtime_dir: runtime_dir.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        manifest_runner_path: manifest
            .runner_path
            .as_ref()
            .map(|path| path.display().to_string()),
        manifest_runner_hash: manifest.runner_hash,
        legacy_runner_path: legacy_runner_path.display().to_string(),
        legacy_runner_hash: hash_file(&legacy_runner_path).map(|file| file.sha256_16),
        bundled_runner_hash: gwt_core::runtime::bundled_project_index_runner_hash(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RuntimeManifestRunner {
    runner_path: Option<PathBuf>,
    runner_hash: Option<String>,
}

fn read_runtime_manifest_runner(runtime_dir: &Path, manifest_path: &Path) -> RuntimeManifestRunner {
    let Ok(bytes) = fs::read(manifest_path) else {
        return RuntimeManifestRunner::default();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return RuntimeManifestRunner::default();
    };
    let runner = value.get("runner");
    let runner_hash = runner
        .and_then(|runner| runner.get("sha256_16"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let runner_path = runner
        .and_then(|runner| runner.get("path"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                runtime_dir.join(path)
            }
        });
    RuntimeManifestRunner {
        runner_path,
        runner_hash,
    }
}

fn collect_recent_log_budget(
    projects_dir: &Path,
    file_limit: usize,
    lines_per_file: usize,
) -> LogBudgetDiagnostics {
    let mut files = Vec::new();
    collect_project_log_files(projects_dir, &mut files);
    files.sort_by_key(|(modified, _)| Reverse(*modified));

    let mut diagnostics = LogBudgetDiagnostics::default();
    for (_modified, path) in files.into_iter().take(file_limit) {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        diagnostics.inspected_files += 1;
        let lines = content.lines().rev().take(lines_per_file);
        count_log_budget_from_lines(lines, &mut diagnostics);
    }
    diagnostics
}

fn collect_project_log_files(dir: &Path, files: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        collect_log_files_in_dir(&path.join("logs"), files);
    }
}

fn collect_log_files_in_dir(log_dir: &Path, files: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("gwt.log.") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        files.push((modified, path));
    }
}

fn count_log_budget_from_lines<'a>(
    lines: impl Iterator<Item = &'a str>,
    diagnostics: &mut LogBudgetDiagnostics,
) {
    for line in lines {
        diagnostics.inspected_lines += 1;
        if line.contains("project index status runner completed for worktree") {
            diagnostics.status_completed_info += 1;
        }
        if line.contains("/internal/hook-live") && line.contains("\"status\":204") {
            diagnostics.hook_live_204_info += 1;
        }
    }
}

fn hash_file(path: &Path) -> Option<FileHash> {
    let bytes = fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let metadata = fs::metadata(path).ok()?;
    Some(FileHash {
        path: path.display().to_string(),
        sha256_16: hex::encode(hasher.finalize())[..16].to_string(),
        size_bytes: metadata.len(),
        modified_unix_seconds: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
    })
}

fn hashes_differ(left: Option<&FileHash>, right: Option<&FileHash>) -> Option<bool> {
    Some(left?.sha256_16 != right?.sha256_16)
}

fn unexpected_error(err: impl ToString) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn parses_diagnostics_cpu_json() {
        assert_eq!(
            parse(&s(&["cpu", "--json"])).unwrap(),
            DiagnosticsCommand::Cpu { json: true }
        );
        assert!(parse(&s(&["cpu"])).is_err());
    }

    #[test]
    fn parses_ps_output_and_classifies_gwt_processes() {
        let text = "\
  935     1 100.2 02:17:18 /Applications/GWT.app/Contents/MacOS/gwt
936 935 78.4 00:00:05 /Users/me/.gwt/runtime/venvs/chroma/bin/python /Users/me/.gwt/runtime/runners/chroma_index_runner-abc.py --action status
1200 1 0.0 00:01:00 /Users/me/project/target/debug/gwtd diagnostics cpu --json
";
        let snapshots = parse_process_snapshots(text);
        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0].pid, 935);
        assert_eq!(snapshots[0].cpu_percent, Some(100.2));
        assert!(is_gwt_process(&snapshots[0]));
        assert!(is_chroma_runner_process(&snapshots[1]));
        assert!(!is_gwt_process(&snapshots[2]));
    }

    #[test]
    fn counts_recent_status_and_hook_live_log_lines() {
        let mut diagnostics = LogBudgetDiagnostics::default();
        count_log_budget_from_lines(
            [
                r#"{"message":"project index status runner completed for worktree"}"#,
                r#"{"method":"POST","path":"/internal/hook-live","status":204}"#,
                r#"{"method":"POST","path":"/internal/hook-live","status":500}"#,
            ]
            .into_iter(),
            &mut diagnostics,
        );

        assert_eq!(diagnostics.inspected_lines, 3);
        assert_eq!(diagnostics.status_completed_info, 1);
        assert_eq!(diagnostics.hook_live_204_info, 1);
    }
}
