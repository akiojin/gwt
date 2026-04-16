//! `gwt update` — manual update check and apply.

use std::io::{self, BufRead, Write};

use gwt_core::update::{is_ci, InstallerKind, PreparedPayload, UpdateManager, UpdateState};

/// Parsed form of `gwt update` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCommand {
    /// Only check and report; do not download or apply.
    CheckOnly,
    /// Check and, with user approval, download and apply.
    Apply,
}

/// Parse `gwt update [--check]` arguments.
pub fn parse_args(args: &[String]) -> UpdateCommand {
    if args.iter().any(|a| a == "--check") {
        UpdateCommand::CheckOnly
    } else {
        UpdateCommand::Apply
    }
}

/// Run the update command.
///
/// Returns the process exit code (0 = success, non-zero = error).
pub fn run(cmd: UpdateCommand) -> i32 {
    if is_ci() {
        println!("Update check skipped in CI environment.");
        return 0;
    }

    let mgr = UpdateManager::new();
    let force = true; // `gwt update` always ignores the TTL cache
    let current_exe = std::env::current_exe().ok();
    let state = mgr.check_for_executable(force, current_exe.as_deref());

    match state {
        UpdateState::UpToDate { .. } => {
            println!("gwt is up to date.");
            0
        }
        UpdateState::Failed { message, .. } => {
            eprintln!("Update check failed: {message}");
            1
        }
        UpdateState::Available {
            current,
            latest,
            asset_url,
            ..
        } => {
            println!("Update available: v{current} → v{latest}");

            if cmd == UpdateCommand::CheckOnly {
                return 0;
            }

            let Some(asset_url) = asset_url else {
                eprintln!("No suitable update asset found for this platform.");
                return 1;
            };

            print!("Apply update now? [Y/n] ");
            let _ = io::stdout().flush();

            let mut line = String::new();
            if io::stdin().lock().read_line(&mut line).is_err() {
                eprintln!("Failed to read input.");
                return 1;
            }
            let answer = line.trim().to_ascii_lowercase();
            if !answer.is_empty() && answer != "y" {
                println!("Update cancelled.");
                return 0;
            }

            println!("Downloading v{latest}...");
            let payload = match mgr.prepare_update(&latest, &asset_url) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Download failed: {e}");
                    return 1;
                }
            };

            let current_exe = match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Failed to locate current executable: {e}");
                    return 1;
                }
            };

            let args_file = match &payload {
                PreparedPayload::PortableBinary { path }
                | PreparedPayload::Installer { path, .. } => {
                    path.parent().map(|d| d.join("restart-args.json"))
                }
            };
            let Some(args_file) = args_file else {
                eprintln!("Invalid payload path.");
                return 1;
            };

            let restart_args: Vec<String> = std::env::args().skip(1).collect();
            if let Err(e) = mgr.write_restart_args_file(&args_file, restart_args) {
                eprintln!("Failed to write restart args: {e}");
                return 1;
            }

            let helper_exe = if cfg!(windows) {
                match mgr.make_helper_copy(&current_exe, &latest) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Failed to create update helper: {e}");
                        return 1;
                    }
                }
            } else {
                current_exe.clone()
            };

            let old_pid = std::process::id();
            let result = match payload {
                PreparedPayload::PortableBinary { path } => mgr.spawn_internal_apply_update(
                    &helper_exe,
                    old_pid,
                    &current_exe,
                    &path,
                    &args_file,
                ),
                PreparedPayload::Installer { path, kind } => mgr.spawn_internal_run_installer(
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
                    println!("Updating to v{latest}... restarting.");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Failed to apply update: {e}");
                    1
                }
            }
        }
    }
}

/// Parse `gwt __internal apply-update` arguments and execute the internal update helper.
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
        eprintln!("gwt __internal apply-update: missing required arguments");
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

/// Parse `gwt __internal run-installer` arguments and execute the installer helper.
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
        eprintln!("gwt __internal run-installer: missing required arguments");
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
    use std::sync::Mutex;

    use super::*;

    // Mutex to serialize tests that mutate the process-global CI environment variable.
    static CI_ENV_MUTEX: Mutex<()> = Mutex::new(());

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
}
