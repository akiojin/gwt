//! `gwtd update` — manual update check and apply.

use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

use gwt_core::update::{is_ci, InstallerKind, PreparedPayload, UpdateManager, UpdateState};

/// Parsed form of `gwtd update` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCommand {
    /// Only check and report; do not download or apply.
    CheckOnly,
    /// Check and, with user approval, download and apply.
    Apply,
}

enum RunOutcome {
    Code(i32),
    ExitSuccess,
}

trait UpdateCliOps {
    fn is_ci(&self) -> bool;
    fn current_exe(&self) -> io::Result<PathBuf>;
    fn current_args(&self) -> Vec<String>;
    fn read_line(&mut self, line: &mut String) -> io::Result<usize>;
    fn write_stdout(&mut self, text: &str);
    fn write_stderr(&mut self, text: &str);
    fn flush_stdout(&mut self) -> io::Result<()>;
    fn check_for_executable(&mut self, force: bool, current_exe: Option<&Path>) -> UpdateState;
    fn prepare_update(&mut self, latest: &str, asset_url: &str) -> Result<PreparedPayload, String>;
    fn write_restart_args_file(&mut self, path: &Path, args: Vec<String>) -> Result<(), String>;
    fn make_helper_copy(&mut self, current_exe: &Path, latest: &str) -> Result<PathBuf, String>;
    fn spawn_internal_apply_update(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        payload: &Path,
        args_file: &Path,
    ) -> Result<(), String>;
    fn spawn_internal_run_installer(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        installer: &Path,
        kind: InstallerKind,
        args_file: &Path,
    ) -> Result<(), String>;
}

struct RealUpdateCliOps {
    mgr: UpdateManager,
}

impl Default for RealUpdateCliOps {
    fn default() -> Self {
        Self {
            mgr: UpdateManager::new(),
        }
    }
}

impl UpdateCliOps for RealUpdateCliOps {
    fn is_ci(&self) -> bool {
        is_ci()
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        std::env::current_exe()
    }

    fn current_args(&self) -> Vec<String> {
        std::env::args().skip(1).collect()
    }

    fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
        io::stdin().lock().read_line(line)
    }

    fn write_stdout(&mut self, text: &str) {
        let _ = write!(io::stdout(), "{text}");
    }

    fn write_stderr(&mut self, text: &str) {
        let _ = write!(io::stderr(), "{text}");
    }

    fn flush_stdout(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }

    fn check_for_executable(&mut self, force: bool, current_exe: Option<&Path>) -> UpdateState {
        self.mgr.check_for_executable(force, current_exe)
    }

    fn prepare_update(&mut self, latest: &str, asset_url: &str) -> Result<PreparedPayload, String> {
        self.mgr.prepare_update(latest, asset_url)
    }

    fn write_restart_args_file(&mut self, path: &Path, args: Vec<String>) -> Result<(), String> {
        self.mgr.write_restart_args_file(path, args)
    }

    fn make_helper_copy(&mut self, current_exe: &Path, latest: &str) -> Result<PathBuf, String> {
        self.mgr.make_helper_copy(current_exe, latest)
    }

    fn spawn_internal_apply_update(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        payload: &Path,
        args_file: &Path,
    ) -> Result<(), String> {
        self.mgr
            .spawn_internal_apply_update(helper_exe, old_pid, current_exe, payload, args_file)
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
        self.mgr.spawn_internal_run_installer(
            helper_exe,
            old_pid,
            current_exe,
            installer,
            kind,
            args_file,
        )
    }
}

/// Parse `gwtd update [--check]` arguments.
pub fn parse_args(args: &[String]) -> UpdateCommand {
    if args.iter().any(|a| a == "--check") {
        UpdateCommand::CheckOnly
    } else {
        UpdateCommand::Apply
    }
}

fn run_with(ops: &mut impl UpdateCliOps, cmd: UpdateCommand) -> RunOutcome {
    if ops.is_ci() {
        ops.write_stdout("Update check skipped in CI environment.\n");
        return RunOutcome::Code(0);
    }

    let force = true; // `gwtd update` always ignores the TTL cache
    let current_exe = ops.current_exe().ok();
    let state = ops.check_for_executable(force, current_exe.as_deref());

    match state {
        UpdateState::UpToDate { .. } => {
            ops.write_stdout("gwt is up to date.\n");
            RunOutcome::Code(0)
        }
        UpdateState::Failed { message, .. } => {
            ops.write_stderr(&format!("Update check failed: {message}\n"));
            RunOutcome::Code(1)
        }
        UpdateState::Available {
            current,
            latest,
            asset_url,
            ..
        } => {
            ops.write_stdout(&format!("Update available: v{current} → v{latest}\n"));

            if cmd == UpdateCommand::CheckOnly {
                return RunOutcome::Code(0);
            }

            let Some(asset_url) = asset_url else {
                ops.write_stderr("No suitable update asset found for this platform.\n");
                return RunOutcome::Code(1);
            };

            ops.write_stdout("Apply update now? [Y/n] ");
            let _ = ops.flush_stdout();

            let mut line = String::new();
            if ops.read_line(&mut line).is_err() {
                ops.write_stderr("Failed to read input.\n");
                return RunOutcome::Code(1);
            }
            let answer = line.trim().to_ascii_lowercase();
            if !answer.is_empty() && answer != "y" {
                ops.write_stdout("Update cancelled.\n");
                return RunOutcome::Code(0);
            }

            ops.write_stdout(&format!("Downloading v{latest}...\n"));
            let payload = match ops.prepare_update(&latest, &asset_url) {
                Ok(payload) => payload,
                Err(error) => {
                    ops.write_stderr(&format!("Download failed: {error}\n"));
                    return RunOutcome::Code(1);
                }
            };

            let current_exe = match ops.current_exe() {
                Ok(path) => path,
                Err(error) => {
                    ops.write_stderr(&format!("Failed to locate current executable: {error}\n"));
                    return RunOutcome::Code(1);
                }
            };

            let args_file = match &payload {
                PreparedPayload::PortableBinary { path }
                | PreparedPayload::Installer { path, .. } => {
                    path.parent().map(|dir| dir.join("restart-args.json"))
                }
            };
            let Some(args_file) = args_file else {
                ops.write_stderr("Invalid payload path.\n");
                return RunOutcome::Code(1);
            };

            if let Err(error) = ops.write_restart_args_file(&args_file, ops.current_args()) {
                ops.write_stderr(&format!("Failed to write restart args: {error}\n"));
                return RunOutcome::Code(1);
            }

            let helper_exe = if cfg!(windows) {
                match ops.make_helper_copy(&current_exe, &latest) {
                    Ok(path) => path,
                    Err(error) => {
                        ops.write_stderr(&format!("Failed to create update helper: {error}\n"));
                        return RunOutcome::Code(1);
                    }
                }
            } else {
                current_exe.clone()
            };

            let old_pid = std::process::id();
            let result = match payload {
                PreparedPayload::PortableBinary { path } => ops.spawn_internal_apply_update(
                    &helper_exe,
                    old_pid,
                    &current_exe,
                    &path,
                    &args_file,
                ),
                PreparedPayload::Installer { path, kind } => ops.spawn_internal_run_installer(
                    &helper_exe,
                    old_pid,
                    &current_exe,
                    &path,
                    kind,
                    &args_file,
                ),
            };

            match result {
                Ok(()) => {
                    ops.write_stdout(&format!("Updating to v{latest}... restarting.\n"));
                    RunOutcome::ExitSuccess
                }
                Err(error) => {
                    ops.write_stderr(&format!("Failed to apply update: {error}\n"));
                    RunOutcome::Code(1)
                }
            }
        }
    }
}

/// Run the update command.
///
/// Returns the process exit code (0 = success, non-zero = error).
pub fn run(cmd: UpdateCommand) -> i32 {
    let mut ops = RealUpdateCliOps::default();
    match run_with(&mut ops, cmd) {
        RunOutcome::Code(code) => code,
        RunOutcome::ExitSuccess => std::process::exit(0),
    }
}

/// Parse `gwtd __internal apply-update` arguments and execute the internal update helper.
///
/// argv format: `--old-pid <pid> --target <path> --source <path> --args-file <path>`
pub fn run_internal_apply_update(args: &[String]) -> i32 {
    let old_pid = parse_flag_u32(args, "--old-pid");
    let target = parse_flag_path(args, "--target");
    let source = parse_flag_path(args, "--source");
    let args_file = parse_flag_path(args, "--args-file");

    let (Some(old_pid), Some(target), Some(source), Some(args_file)) =
        (old_pid, target, source, args_file)
    else {
        eprintln!("gwtd __internal apply-update: missing required arguments");
        eprintln!("  Usage: --old-pid <pid> --target <path> --source <path> --args-file <path>");
        return 1;
    };

    match gwt_core::update::internal_apply_update(old_pid, &target, &source, &args_file) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("apply-update failed: {e}");
            1
        }
    }
}

/// Parse `gwtd __internal run-installer` arguments and execute the installer helper.
///
/// argv format: `--old-pid <pid> --target <path> --installer <path> --installer-kind <kind> --args-file <path>`
pub fn run_internal_run_installer(args: &[String]) -> i32 {
    let old_pid = parse_flag_u32(args, "--old-pid");
    let target = parse_flag_path(args, "--target");
    let installer = parse_flag_path(args, "--installer");
    let args_file = parse_flag_path(args, "--args-file");
    let kind_str = parse_flag_str(args, "--installer-kind");

    let kind = match kind_str.as_deref() {
        Some("mac_dmg") => InstallerKind::MacDmg,
        Some("mac_pkg") => InstallerKind::MacPkg,
        Some("windows_msi") => InstallerKind::WindowsMsi,
        other => {
            eprintln!("Unknown installer kind: {other:?}");
            return 1;
        }
    };

    let (Some(old_pid), Some(target), Some(installer), Some(args_file)) =
        (old_pid, target, installer, args_file)
    else {
        eprintln!("gwtd __internal run-installer: missing required arguments");
        return 1;
    };

    match gwt_core::update::internal_run_installer(old_pid, &target, &installer, kind, &args_file) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("run-installer failed: {e}");
            1
        }
    }
}

fn parse_flag_str(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn parse_flag_u32(args: &[String], flag: &str) -> Option<u32> {
    parse_flag_str(args, flag).and_then(|s| s.parse().ok())
}

fn parse_flag_path(args: &[String], flag: &str) -> Option<std::path::PathBuf> {
    parse_flag_str(args, flag).map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Mutex};

    use super::*;

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

        fn check_for_executable(
            &mut self,
            _force: bool,
            _current_exe: Option<&Path>,
        ) -> UpdateState {
            self.state.clone()
        }

        fn prepare_update(
            &mut self,
            _latest: &str,
            _asset_url: &str,
        ) -> Result<PreparedPayload, String> {
            self.prepare_update_result.clone()
        }

        fn write_restart_args_file(
            &mut self,
            path: &Path,
            args: Vec<String>,
        ) -> Result<(), String> {
            self.restart_args_file = Some(path.to_path_buf());
            self.restart_args = args;
            self.write_restart_args_result.clone()
        }

        fn make_helper_copy(
            &mut self,
            current_exe: &Path,
            latest: &str,
        ) -> Result<PathBuf, String> {
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
        assert_eq!(parse_args(&args), UpdateCommand::Apply);
    }

    #[test]
    fn parse_args_check_flag() {
        let args = vec!["--check".to_string()];
        assert_eq!(parse_args(&args), UpdateCommand::CheckOnly);
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
        let code = run(UpdateCommand::CheckOnly);
        std::env::remove_var("CI");
        assert_eq!(code, 0);
    }

    #[test]
    fn run_apply_returns_zero_in_ci() {
        let _guard = CI_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("CI", "true");
        let code = run(UpdateCommand::Apply);
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
            run_internal_run_installer(&[
                "--installer-kind".to_string(),
                "windows_msi".to_string(),
            ]),
            1
        );
    }

    #[test]
    fn run_with_covers_ci_and_non_available_states() {
        let mut ci = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        ci.is_ci = true;
        assert!(matches!(
            run_with(&mut ci, UpdateCommand::Apply),
            RunOutcome::Code(0)
        ));
        assert!(ci.stdout.contains("skipped in CI"));

        let mut up_to_date = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        up_to_date.state = UpdateState::UpToDate {
            checked_at: Some(chrono::Utc::now()),
        };
        assert!(matches!(
            run_with(&mut up_to_date, UpdateCommand::Apply),
            RunOutcome::Code(0)
        ));
        assert!(up_to_date.stdout.contains("up to date"));

        let mut failed = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        failed.state = UpdateState::Failed {
            message: "network down".to_string(),
            failed_at: chrono::Utc::now(),
        };
        assert!(matches!(
            run_with(&mut failed, UpdateCommand::Apply),
            RunOutcome::Code(1)
        ));
        assert!(failed.stderr.contains("network down"));
    }

    #[test]
    fn run_with_covers_check_only_cancel_and_missing_asset_paths() {
        let mut check_only = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        assert!(matches!(
            run_with(&mut check_only, UpdateCommand::CheckOnly),
            RunOutcome::Code(0)
        ));
        assert!(check_only.stdout.contains("Update available"));
        assert!(check_only.apply_calls.is_empty());

        let mut cancelled = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        cancelled.input_line = Ok("n\n".to_string());
        assert!(matches!(
            run_with(&mut cancelled, UpdateCommand::Apply),
            RunOutcome::Code(0)
        ));
        assert!(cancelled.stdout.contains("Update cancelled"));

        let mut missing_asset = FakeUpdateCliOps::available(None);
        assert!(matches!(
            run_with(&mut missing_asset, UpdateCommand::Apply),
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
            run_with(&mut prepare_error, UpdateCommand::Apply),
            RunOutcome::Code(1)
        ));
        assert!(prepare_error.stderr.contains("Download failed"));

        let mut current_exe_error =
            FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        current_exe_error.current_exe = Err(io::Error::new(io::ErrorKind::NotFound, "missing"));
        assert!(matches!(
            run_with(&mut current_exe_error, UpdateCommand::Apply),
            RunOutcome::Code(1)
        ));
        assert!(current_exe_error
            .stderr
            .contains("Failed to locate current executable"));

        let mut restart_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        restart_error.write_restart_args_result = Err("disk full".to_string());
        assert!(matches!(
            run_with(&mut restart_error, UpdateCommand::Apply),
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
            run_with(&mut apply, UpdateCommand::Apply),
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
            run_with(&mut apply_error, UpdateCommand::Apply),
            RunOutcome::Code(1)
        ));
        assert!(apply_error.stderr.contains("Failed to apply update"));

        let mut helper_error = FakeUpdateCliOps::available(Some("https://example.test/gwt.zip"));
        helper_error.make_helper_copy_result = Err("copy failed".to_string());
        if cfg!(windows) {
            assert!(matches!(
                run_with(&mut helper_error, UpdateCommand::Apply),
                RunOutcome::Code(1)
            ));
            assert!(helper_error
                .stderr
                .contains("Failed to create update helper"));
        } else {
            assert!(matches!(
                run_with(&mut helper_error, UpdateCommand::Apply),
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
            run_with(&mut installer, UpdateCommand::Apply),
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
            run_with(&mut installer_error, UpdateCommand::Apply),
            RunOutcome::Code(1)
        ));
        assert!(installer_error.stderr.contains("Failed to apply update"));
    }
}
