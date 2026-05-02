//! Test suite for `cli::update` (SPEC-1942 SC-027 split). Lives as a
//! sibling file to `mod.rs` and is included via `#[cfg(test)] mod tests;`.

#![cfg(test)]

use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use super::ops::UpdateCliOps;
use super::*;
use gwt_core::update::{InstallerKind, PreparedPayload, UpdateState};

// Mutex to serialize tests that mutate the process-global CI environment variable.
static CI_ENV_MUTEX: Mutex<()> = Mutex::new(());

struct FakeUpdateCliOps {
    is_ci: bool,
    state: UpdateState,
    current_exe: io::Result<PathBuf>,
    current_args: Vec<String>,
    input_line: io::Result<String>,
    stdout: String,
    stderr: String,
    prepare_update_result: Result<PreparedPayload, String>,
    write_restart_args_result: Result<(), String>,
    make_helper_copy_result: Result<PathBuf, String>,
    spawn_apply_result: Result<(), String>,
    spawn_installer_result: Result<(), String>,
    restart_args_file: Option<PathBuf>,
    restart_args: Vec<String>,
    helper_copy_calls: Vec<(PathBuf, String)>,
    apply_calls: Vec<(PathBuf, u32, PathBuf, PathBuf, PathBuf)>,
    installer_calls: Vec<(PathBuf, u32, PathBuf, PathBuf, InstallerKind, PathBuf)>,
}

impl FakeUpdateCliOps {
    fn available(asset_url: Option<&str>) -> Self {
        Self {
            is_ci: false,
            state: UpdateState::Available {
                current: "1.0.0".to_string(),
                latest: "1.1.0".to_string(),
                release_url: "https://example.test/release".to_string(),
                asset_url: asset_url.map(str::to_string),
                checked_at: chrono::Utc::now(),
            },
            current_exe: Ok(PathBuf::from("C:/gwt/gwt.exe")),
            current_args: vec!["update".to_string(), "--check".to_string()],
            input_line: Ok("y\n".to_string()),
            stdout: String::new(),
            stderr: String::new(),
            prepare_update_result: Ok(PreparedPayload::PortableBinary {
                path: PathBuf::from("C:/updates/gwt.exe"),
            }),
            write_restart_args_result: Ok(()),
            make_helper_copy_result: Ok(PathBuf::from("C:/updates/gwt-helper.exe")),
            spawn_apply_result: Ok(()),
            spawn_installer_result: Ok(()),
            restart_args_file: None,
            restart_args: Vec::new(),
            helper_copy_calls: Vec::new(),
            apply_calls: Vec::new(),
            installer_calls: Vec::new(),
        }
    }
}

impl UpdateCliOps for FakeUpdateCliOps {
    fn is_ci(&self) -> bool {
        self.is_ci
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        match &self.current_exe {
            Ok(path) => Ok(path.clone()),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }

    fn current_args(&self) -> Vec<String> {
        self.current_args.clone()
    }

    fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
        match &self.input_line {
            Ok(value) => {
                line.push_str(value);
                Ok(value.len())
            }
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }

    fn write_stdout(&mut self, text: &str) {
        self.stdout.push_str(text);
    }

    fn write_stderr(&mut self, text: &str) {
        self.stderr.push_str(text);
    }

    fn flush_stdout(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn check_for_executable(&mut self, _force: bool, _current_exe: Option<&Path>) -> UpdateState {
        self.state.clone()
    }

    fn prepare_update(
        &mut self,
        _latest: &str,
        _asset_url: &str,
    ) -> Result<PreparedPayload, String> {
        self.prepare_update_result.clone()
    }

    fn write_restart_args_file(&mut self, path: &Path, args: Vec<String>) -> Result<(), String> {
        self.restart_args_file = Some(path.to_path_buf());
        self.restart_args = args;
        self.write_restart_args_result.clone()
    }

    fn make_helper_copy(&mut self, current_exe: &Path, latest: &str) -> Result<PathBuf, String> {
        self.helper_copy_calls
            .push((current_exe.to_path_buf(), latest.to_string()));
        self.make_helper_copy_result.clone()
    }

    fn spawn_internal_apply_update(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        payload: &Path,
        args_file: &Path,
    ) -> Result<(), String> {
        self.apply_calls.push((
            helper_exe.to_path_buf(),
            old_pid,
            current_exe.to_path_buf(),
            payload.to_path_buf(),
            args_file.to_path_buf(),
        ));
        self.spawn_apply_result.clone()
    }

    fn spawn_internal_run_installer(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        installer: &Path,
        kind: InstallerKind,
        args_file: &Path,
    ) -> Result<(), String> {
        self.installer_calls.push((
            helper_exe.to_path_buf(),
            old_pid,
            current_exe.to_path_buf(),
            installer.to_path_buf(),
            kind,
            args_file.to_path_buf(),
        ));
        self.spawn_installer_result.clone()
    }
}

#[test]
fn parse_args_defaults_to_apply() {
    let args: Vec<String> = vec![];
    assert_eq!(parse_args(&args), UpdateRunMode::Apply);
}

#[test]
fn parse_args_check_flag() {
    let args = vec!["--check".to_string()];
    assert_eq!(parse_args(&args), UpdateRunMode::CheckOnly);
}

#[test]
fn parse_flag_str_extracts_value() {
    let args = vec!["--old-pid".to_string(), "123".to_string()];
    assert_eq!(parse_flag_str(&args, "--old-pid"), Some("123".to_string()));
}

#[test]
fn parse_flag_u32_parses_number() {
    let args = vec!["--old-pid".to_string(), "456".to_string()];
    assert_eq!(parse_flag_u32(&args, "--old-pid"), Some(456u32));
}

#[test]
fn run_check_only_returns_zero_in_ci() {
    let _guard = CI_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var("CI", "true");
    let code = run(UpdateRunMode::CheckOnly);
    std::env::remove_var("CI");
    assert_eq!(code, 0);
}

#[test]
fn run_apply_returns_zero_in_ci() {
    let _guard = CI_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var("CI", "true");
    let code = run(UpdateRunMode::Apply);
    std::env::remove_var("CI");
    assert_eq!(code, 0);
}

#[test]
fn parse_flag_path_extracts_path_and_missing_flags_return_none() {
    let args = vec![
        "--target".to_string(),
        "/tmp/gwt".to_string(),
        "--old-pid".to_string(),
        "not-a-number".to_string(),
    ];
    assert_eq!(
        parse_flag_path(&args, "--target"),
        Some(std::path::PathBuf::from("/tmp/gwt"))
    );
    assert_eq!(parse_flag_str(&args, "--missing"), None);
    assert_eq!(parse_flag_u32(&args, "--old-pid"), None);
}

#[test]
fn internal_update_helpers_reject_missing_or_invalid_arguments() {
    assert_eq!(run_internal_apply_update(&[]), 1);
    assert_eq!(
        run_internal_run_installer(&[
            "--old-pid".to_string(),
            "1".to_string(),
            "--target".to_string(),
            "bin/gwt".to_string(),
            "--installer".to_string(),
            "installer.msi".to_string(),
            "--args-file".to_string(),
            "restart.json".to_string(),
            "--installer-kind".to_string(),
            "unknown".to_string(),
        ]),
        1
    );
    assert_eq!(
        run_internal_run_installer(&["--installer-kind".to_string(), "windows_msi".to_string(),]),
        1
    );
}

#[test]
fn run_with_covers_ci_and_non_available_states() {
    let mut ci = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    ci.is_ci = true;
    assert!(matches!(
        run_with(&mut ci, UpdateRunMode::Apply),
        RunOutcome::Code(0)
    ));
    assert!(ci.stdout.contains("skipped in CI"));

    let mut up_to_date = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    up_to_date.state = UpdateState::UpToDate {
        checked_at: Some(chrono::Utc::now()),
    };
    assert!(matches!(
        run_with(&mut up_to_date, UpdateRunMode::Apply),
        RunOutcome::Code(0)
    ));
    assert!(up_to_date.stdout.contains("up to date"));

    let mut failed = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    failed.state = UpdateState::Failed {
        message: "network down".to_string(),
        failed_at: chrono::Utc::now(),
    };
    assert!(matches!(
        run_with(&mut failed, UpdateRunMode::Apply),
        RunOutcome::Code(1)
    ));
    assert!(failed.stderr.contains("network down"));
}

#[test]
fn run_with_covers_check_only_cancel_and_missing_asset_paths() {
    let mut check_only = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    assert!(matches!(
        run_with(&mut check_only, UpdateRunMode::CheckOnly),
        RunOutcome::Code(0)
    ));
    assert!(check_only.stdout.contains("Update available"));
    assert!(check_only.apply_calls.is_empty());

    let mut cancelled = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    cancelled.input_line = Ok("n\n".to_string());
    assert!(matches!(
        run_with(&mut cancelled, UpdateRunMode::Apply),
        RunOutcome::Code(0)
    ));
    assert!(cancelled.stdout.contains("Update cancelled"));

    let mut missing_asset = FakeUpdateCliOps::available(None);
    assert!(matches!(
        run_with(&mut missing_asset, UpdateRunMode::Apply),
        RunOutcome::Code(1)
    ));
    assert!(missing_asset
        .stderr
        .contains("No suitable update asset found"));
}

#[test]
fn run_with_covers_prepare_current_exe_and_restart_arg_failures() {
    let mut prepare_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    prepare_error.prepare_update_result = Err("download broke".to_string());
    assert!(matches!(
        run_with(&mut prepare_error, UpdateRunMode::Apply),
        RunOutcome::Code(1)
    ));
    assert!(prepare_error.stderr.contains("Download failed"));

    let mut current_exe_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    current_exe_error.current_exe = Err(io::Error::new(io::ErrorKind::NotFound, "missing"));
    assert!(matches!(
        run_with(&mut current_exe_error, UpdateRunMode::Apply),
        RunOutcome::Code(1)
    ));
    assert!(current_exe_error
        .stderr
        .contains("Failed to locate current executable"));

    let mut restart_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    restart_error.write_restart_args_result = Err("disk full".to_string());
    assert!(matches!(
        run_with(&mut restart_error, UpdateRunMode::Apply),
        RunOutcome::Code(1)
    ));
    assert!(restart_error
        .stderr
        .contains("Failed to write restart args"));
    assert_eq!(restart_error.restart_args, vec!["update", "--check"]);
    assert_eq!(
        restart_error.restart_args_file.as_deref(),
        Some(Path::new("C:/updates/restart-args.json"))
    );
}

#[test]
fn run_with_covers_helper_copy_and_spawn_paths() {
    let mut apply = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    assert!(matches!(
        run_with(&mut apply, UpdateRunMode::Apply),
        RunOutcome::ExitSuccess
    ));
    if cfg!(windows) {
        assert_eq!(apply.helper_copy_calls.len(), 1);
    } else {
        assert!(apply.helper_copy_calls.is_empty());
    }
    assert_eq!(apply.apply_calls.len(), 1);
    assert!(apply.stdout.contains("restarting"));

    let mut apply_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    apply_error.spawn_apply_result = Err("spawn failed".to_string());
    assert!(matches!(
        run_with(&mut apply_error, UpdateRunMode::Apply),
        RunOutcome::Code(1)
    ));
    assert!(apply_error.stderr.contains("Failed to apply update"));

    let mut helper_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
    helper_error.make_helper_copy_result = Err("copy failed".to_string());
    if cfg!(windows) {
        assert!(matches!(
            run_with(&mut helper_error, UpdateRunMode::Apply),
            RunOutcome::Code(1)
        ));
        assert!(helper_error
            .stderr
            .contains("Failed to create update helper"));
    } else {
        assert!(matches!(
            run_with(&mut helper_error, UpdateRunMode::Apply),
            RunOutcome::ExitSuccess
        ));
        assert!(helper_error.helper_copy_calls.is_empty());
    }

    let mut installer = FakeUpdateCliOps::available(Some("https://example.test/gwt.msi"));
    installer.prepare_update_result = Ok(PreparedPayload::Installer {
        path: PathBuf::from("C:/updates/gwt.msi"),
        kind: InstallerKind::WindowsMsi,
    });
    assert!(matches!(
        run_with(&mut installer, UpdateRunMode::Apply),
        RunOutcome::ExitSuccess
    ));
    assert_eq!(installer.installer_calls.len(), 1);

    let mut installer_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.msi"));
    installer_error.prepare_update_result = Ok(PreparedPayload::Installer {
        path: PathBuf::from("C:/updates/gwt.msi"),
        kind: InstallerKind::WindowsMsi,
    });
    installer_error.spawn_installer_result = Err("installer failed".to_string());
    assert!(matches!(
        run_with(&mut installer_error, UpdateRunMode::Apply),
        RunOutcome::Code(1)
    ));
    assert!(installer_error.stderr.contains("Failed to apply update"));
}
