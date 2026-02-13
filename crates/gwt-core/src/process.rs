//! Process helpers for launching external commands.
//!
//! On Windows, GUI applications should spawn child processes without creating
//! transient console windows.

use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Build a command configured for GUI-friendly execution.
pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    configure_no_window(&mut cmd);
    cmd
}

/// Build a Git command configured for GUI-friendly execution.
pub fn git_command() -> Command {
    command("git")
}

/// Apply platform-specific no-window behavior.
pub fn configure_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_command_targets_git_binary() {
        assert_eq!(git_command().get_program(), "git");
    }

    #[test]
    fn configure_no_window_is_safe_on_all_platforms() {
        let mut cmd = command("git");
        configure_no_window(&mut cmd);
    }
}
