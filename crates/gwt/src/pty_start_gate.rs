//! Resolution of the trusted local binary that hosts a bound launch's PTY
//! start gate.
//!
//! Windows links the GUI front door (`gwt.exe`) with
//! `windows_subsystem = "windows"`. `CreateProcess` only attaches console
//! subsystem images to the pseudoconsole handed over through
//! `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, so a start gate hosted by `gwt.exe`
//! runs with no console and NULL std handles. The target it then releases
//! inherits nothing usable and Windows allocates a *new* console for it, which
//! Windows 11 hands to the configured default terminal application — the agent
//! TUI renders in an external Windows Terminal window while the gwt pane stays
//! blank (issue #3631).
//!
//! The console subsystem `gwtd` companion is installed next to `gwt` by every
//! distribution path, so it can host the gate and keep the released agent
//! inside the pane's ConPTY.

use std::path::{Path, PathBuf};

/// `argv[1]` marker that switches a gwt binary into PTY start-gate mode.
pub const PTY_START_GATE_ARG: &str = "__internal-pty-start-gate";

/// Platform whose console rules decide which binary may host the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateHostPlatform {
    Windows,
    Posix,
}

impl GateHostPlatform {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
}

/// Resolve the start-gate program for the running executable.
pub fn resolve_pty_start_gate_program(current_exe: &Path) -> Result<PathBuf, String> {
    resolve_pty_start_gate_program_for_platform(current_exe, GateHostPlatform::current(), &|path| {
        path.is_file()
    })
}

/// Resolve the start-gate program using explicit platform rules.
///
/// The explicit form keeps the Windows console-subsystem requirement testable
/// on every development host.
pub fn resolve_pty_start_gate_program_for_platform(
    current_exe: &Path,
    platform: GateHostPlatform,
    is_file: &dyn Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    // POSIX images have no subsystem split, and the gate replaces itself with
    // the target through `exec` there, which must preserve the gated PID.
    if platform == GateHostPlatform::Posix {
        return Ok(current_exe.to_path_buf());
    }

    let companion = crate::cli::gwtd_resolver::gwtd_companion_path(current_exe);
    if is_file(&companion) {
        return Ok(companion);
    }
    Err(format!(
        "the console-subsystem gwtd companion is required to host the PTY start gate but '{}' does not exist; reinstall gwt so gwtd ships next to '{}'",
        companion.display(),
        current_exe.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_hosts_the_gate_in_the_running_executable() {
        let resolved = resolve_pty_start_gate_program_for_platform(
            Path::new("/opt/gwt/bin/gwt"),
            GateHostPlatform::Posix,
            &|_| false,
        )
        .expect("POSIX gate resolution");

        assert_eq!(resolved, PathBuf::from("/opt/gwt/bin/gwt"));
    }

    /// A `C:\...` literal is a single path component on POSIX hosts, so
    /// `with_file_name` would replace the whole string and the sibling rule
    /// under test would never be exercised there. The rule is about the
    /// install directory, not about a separator, so build the layout with
    /// `join` and let every development host run these.
    fn install_dir() -> PathBuf {
        PathBuf::from("gwt-install-dir")
    }

    #[test]
    fn windows_hosts_the_gate_in_the_gwtd_companion() {
        let front_door = install_dir().join("gwt.exe");
        let companion = install_dir().join("gwtd.exe");
        let resolved = resolve_pty_start_gate_program_for_platform(
            &front_door,
            GateHostPlatform::Windows,
            &|path| path == companion,
        )
        .expect("Windows gate resolution");

        assert_eq!(resolved, companion);
    }

    #[test]
    fn windows_keeps_an_already_console_subsystem_host() {
        let companion = install_dir().join("gwtd.exe");
        let resolved = resolve_pty_start_gate_program_for_platform(
            &companion,
            GateHostPlatform::Windows,
            &|path| path == companion,
        )
        .expect("Windows gate resolution");

        assert_eq!(resolved, companion);
    }

    #[test]
    fn windows_refuses_to_fall_back_to_the_gui_front_door() {
        let error = resolve_pty_start_gate_program_for_platform(
            &install_dir().join("gwt.exe"),
            GateHostPlatform::Windows,
            &|_| false,
        )
        .expect_err("a missing gwtd companion must fail loudly");

        assert!(error.contains("gwtd.exe"), "{error}");
        assert!(error.contains("PTY start gate"), "{error}");
    }
}
