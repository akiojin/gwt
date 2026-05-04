//! `gwtd update` — manual update check and apply.
//!
//! SPEC-1942 SC-027 split: `mod.rs` keeps the public CLI entry points
//! (`UpdateRunMode`, `parse_args`, `run`, `run_internal_apply_update`,
//! `run_internal_run_installer`). The `UpdateCliOps` trait and its production
//! `RealUpdateCliOps` impl live in the private sibling `ops` module; tests
//! live in the `#[cfg(test)] mod tests` sibling.

mod ops;
#[cfg(test)]
mod tests;

use gwt_core::update::{InstallerKind, PreparedPayload, UpdateState};

use ops::{RealUpdateCliOps, UpdateCliOps};

/// Parsed form of `gwtd update` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateRunMode {
    /// Only check and report; do not download or apply.
    CheckOnly,
    /// Check and, with user approval, download and apply.
    Apply,
}

enum RunOutcome {
    Code(i32),
    ExitSuccess,
}

/// Parse `gwtd update [--check]` arguments.
pub fn parse_args(args: &[String]) -> UpdateRunMode {
    if args.iter().any(|a| a == "--check") {
        UpdateRunMode::CheckOnly
    } else {
        UpdateRunMode::Apply
    }
}

fn run_with(ops: &mut impl UpdateCliOps, cmd: UpdateRunMode) -> RunOutcome {
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

            if cmd == UpdateRunMode::CheckOnly {
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
pub fn run(cmd: UpdateRunMode) -> i32 {
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
