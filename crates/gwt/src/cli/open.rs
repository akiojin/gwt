//! SPEC #2920 FR-006 / Phase 6 — `gwt open` CLI verb.
//!
//! Discovers the running tray-resident process via its single-instance
//! lock file (Phase 3) and launches the OS default browser at the
//! embedded server URL. Designed so Linux users on SNI-poor DEs
//! (GNOME 3.26+) can open the UI from a shell even when the tray icon
//! is not visible (SPEC #2920 Q7 fallback).
//!
//! Exit codes:
//! - 0: launched a browser. The spawned launcher is detached so the
//!   command returns immediately.
//! - 1: no running tray instance was found (the lock file is missing or
//!   has no URL yet). The user is asked to start `gwt` first.
//! - 2: argv parse error (unknown flag / extra argument).

use std::path::Path;

use gwt_github::SpecOpsError;

use super::tray::lock::{current_user_id, lock_path, TrayLockFile};
use super::{CliEnv, CliParseError};

/// `gwt open` takes no positional arguments today. Future Phase 6
/// follow-ups may add `--url <url>` for explicit overrides; SPEC #2920
/// keeps that out of scope for v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenArgs;

/// Parse `gwt open [...]` after the verb has already been stripped.
pub fn parse_args(args: &[String]) -> Result<super::CliCommand, CliParseError> {
    if let Some(extra) = args.iter().find(|arg| !arg.is_empty()) {
        return Err(CliParseError::UnknownSubcommand(extra.clone()));
    }
    Ok(super::CliCommand::Open(OpenArgs))
}

/// Run `gwt open`. Resolves the tray lock file under
/// `<gwt_home>/run/tray-<user_id>.lock`, reads its URL, and spawns the
/// OS default browser. Errors are written to `out` (which the caller
/// then prints) so the test harness can capture stderr without forking
/// processes.
pub fn run<E: CliEnv>(
    _env: &mut E,
    _args: OpenArgs,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let gwt_home = gwt_core::paths::gwt_home();
    run_with_home(&gwt_home, out, &spawn_default_browser_launcher)
}

/// Inner entry point used by both the CLI dispatch path and the unit
/// tests. The launcher is injected so tests can spy on which URL was
/// requested without spawning a real browser.
pub(crate) fn run_with_home(
    gwt_home: &Path,
    out: &mut String,
    launcher: &dyn Fn(&str) -> std::io::Result<()>,
) -> Result<i32, SpecOpsError> {
    let user_id = current_user_id();
    let path = lock_path(gwt_home, &user_id);
    if !path.exists() {
        out.push_str(&format!(
            "gwt open: no running gwt instance found (expected lock at {})\n",
            path.display()
        ));
        out.push_str(
            "hint: launch `gwt` (with no arguments) to start the tray-resident process first.\n",
        );
        return Ok(1);
    }
    let payload = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            out.push_str(&format!(
                "gwt open: could not read tray lock at {}: {error}\n",
                path.display()
            ));
            return Ok(1);
        }
    };
    if payload.trim().is_empty() {
        out.push_str(&format!(
            "gwt open: tray lock at {} is empty (server still starting?)\n",
            path.display()
        ));
        return Ok(1);
    }
    let lock: TrayLockFile = match serde_json::from_str(&payload) {
        Ok(value) => value,
        Err(error) => {
            out.push_str(&format!(
                "gwt open: tray lock at {} is corrupt: {error}\n",
                path.display()
            ));
            return Ok(1);
        }
    };
    if lock.url.trim().is_empty() {
        out.push_str(&format!(
            "gwt open: tray lock at {} has no URL yet (server still binding?)\n",
            path.display()
        ));
        return Ok(1);
    }
    match launcher(&lock.url) {
        Ok(()) => Ok(0),
        Err(error) => {
            out.push_str(&format!(
                "gwt open: could not launch browser for {}: {error}\n",
                lock.url
            ));
            Ok(1)
        }
    }
}

/// Spawn the OS-native default browser at `url`. The launcher is
/// detached and a reaper thread waits on the child so repeated
/// invocations do not accumulate zombies on Unix.
fn spawn_default_browser_launcher(url: &str) -> std::io::Result<()> {
    use std::process::Command;
    let child = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()?
    } else if cfg!(target_os = "windows") {
        // The empty "" before the URL is required by `start` so a URL
        // beginning with quoted text is not treated as a window title.
        Command::new("cmd").args(["/C", "start", "", url]).spawn()?
    } else {
        Command::new("xdg-open").arg(url).spawn()?
    };
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::Mutex;
    use tempfile::TempDir;

    use crate::cli::TestEnv;

    fn write_lock(tmp: &TempDir, url: &str) {
        let user_id = current_user_id();
        let path = lock_path(tmp.path(), &user_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload = TrayLockFile {
            pid: std::process::id(),
            url: url.to_string(),
            started_at: Utc::now(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
    }

    #[test]
    fn parse_args_accepts_zero_arguments() {
        let parsed = parse_args(&[]).expect("empty argv parses");
        assert!(matches!(parsed, super::super::CliCommand::Open(OpenArgs)));
    }

    #[test]
    fn parse_args_rejects_unknown_extras() {
        let argv = vec!["--no-such-flag".to_string()];
        let err = parse_args(&argv).expect_err("unknown flag must error");
        assert!(matches!(err, CliParseError::UnknownSubcommand(flag) if flag == "--no-such-flag"));
    }

    #[test]
    fn run_returns_exit_1_when_lock_file_is_missing() {
        let tmp = TempDir::new().unwrap();
        let mut out = String::new();
        let launched: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let launcher = |url: &str| {
            launched.lock().unwrap().push(url.to_string());
            Ok(())
        };
        let exit = run_with_home(tmp.path(), &mut out, &launcher)
            .expect("run_with_home should not error on missing lock");
        assert_eq!(exit, 1);
        assert!(launched.lock().unwrap().is_empty());
        assert!(out.contains("no running gwt instance found"));
    }

    #[test]
    fn run_returns_exit_1_when_lock_has_no_url() {
        let tmp = TempDir::new().unwrap();
        write_lock(&tmp, "");
        let mut out = String::new();
        let launcher = |_: &str| -> std::io::Result<()> { Ok(()) };
        let exit = run_with_home(tmp.path(), &mut out, &launcher)
            .expect("run_with_home should not error on empty URL");
        assert_eq!(exit, 1);
        assert!(out.contains("has no URL yet"));
    }

    #[test]
    fn run_invokes_launcher_with_lock_url() {
        let tmp = TempDir::new().unwrap();
        write_lock(&tmp, "http://127.0.0.1:55555/");
        let mut out = String::new();
        let launched: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let launcher = |url: &str| {
            launched.lock().unwrap().push(url.to_string());
            Ok(())
        };
        let exit = run_with_home(tmp.path(), &mut out, &launcher).expect("launcher should run");
        assert_eq!(exit, 0);
        assert_eq!(
            launched.lock().unwrap().as_slice(),
            ["http://127.0.0.1:55555/".to_string()].as_slice()
        );
        // Successful launch produces no error message.
        assert!(out.is_empty(), "expected silent success but got: {out:?}");
    }

    #[test]
    fn run_through_dispatch_facade_compiles_with_cli_env() {
        // Sanity-check the dispatch surface: `run(env, args, out)` must
        // be invocable with the standard `TestEnv` even though Phase 6
        // does not exercise any env capabilities. Catches signature
        // drift between the public CLI dispatcher and `open::run`.
        //
        // Scoped HOME (#3022): with the developer's real home this test used
        // to read the production tray lock and launch the OS browser at the
        // running gwt URL on every `cargo test`. An isolated home has no
        // lock, so the run deterministically exits 1 without spawning
        // anything.
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let previous_home = std::env::var_os("HOME");
        let previous_profile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("USERPROFILE", tmp.path());

        let mut env = TestEnv::new(std::path::PathBuf::from("cache-root"));
        let mut out = String::new();
        let exit = run(&mut env, OpenArgs, &mut out).unwrap();

        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match previous_profile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        assert_eq!(exit, 1, "isolated home has no tray lock");
    }
}
