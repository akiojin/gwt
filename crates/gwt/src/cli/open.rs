//! SPEC #2920 FR-006 / Phase 6 — `gwt open [path]` CLI verb.
//!
//! Discovers the running tray-resident process via its single-instance
//! lock file (Phase 3) and launches the OS default browser at the
//! embedded server URL. Designed so Linux users on SNI-poor DEs
//! (GNOME 3.26+) can open the UI from a shell even when the tray icon
//! is not visible (SPEC #2920 Q7 fallback).
//!
//! Issue #4538 AC-4: with a `path`, the running process first opens that
//! Project through its authenticated local control request and returns the
//! ProjectKey; only then is the per-project URL `/p/<key>` launched. A
//! failed request never launches a browser.
//!
//! Exit codes:
//! - 0: launched a browser. The spawned launcher is detached so the
//!   command returns immediately.
//! - 1: no running tray instance was found (the lock file is missing or
//!   has no URL yet), or the running instance refused to open the path.
//! - 2: argv parse error (unknown flag / extra argument).

use std::path::{Path, PathBuf};
use std::time::Duration;

use gwt_github::SpecOpsError;

use super::tray::lock::{current_user_id, lock_path, TrayLockFile};
use super::{CliEnv, CliParseError};
use crate::project_open_control::{
    ProjectOpenControlErrorBody, ProjectOpenControlRequest, ProjectOpenControlResponse,
    PROJECT_OPEN_CONTROL_PATH,
};

/// `gwt open [path]`. Without a path the Hub (root URL) opens, exactly as
/// before Issue #4538.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenArgs {
    pub path: Option<PathBuf>,
}

/// Parse `gwt open [...]` after the verb has already been stripped.
pub fn parse_args(args: &[String]) -> Result<super::CliCommand, CliParseError> {
    let mut positional = args.iter().filter(|arg| !arg.is_empty());
    let path = match positional.next() {
        Some(flag) if flag.starts_with('-') => {
            return Err(CliParseError::UnknownSubcommand(flag.clone()))
        }
        Some(path) => Some(PathBuf::from(path)),
        None => None,
    };
    if let Some(extra) = positional.next() {
        return Err(CliParseError::UnknownSubcommand(extra.clone()));
    }
    Ok(super::CliCommand::Open(OpenArgs { path }))
}

/// Sends the control request: `(server_url, control_token, absolute_path)`.
pub(crate) type ProjectOpenRequester<'a> =
    dyn Fn(&str, &str, &Path) -> Result<ProjectOpenControlResponse, String> + 'a;

/// Run `gwt open`. Resolves the tray lock file under
/// `<gwt_home>/run/tray-<user_id>.lock`, reads its URL, and spawns the
/// OS default browser. Errors are written to `out` (which the caller
/// then prints) so the test harness can capture stderr without forking
/// processes.
pub fn run<E: CliEnv>(_env: &mut E, args: OpenArgs, out: &mut String) -> Result<i32, SpecOpsError> {
    let gwt_home = gwt_core::paths::gwt_home();
    run_with_home(
        &gwt_home,
        args.path.as_deref(),
        out,
        &spawn_default_browser_launcher,
        &send_project_open_request,
    )
}

/// Inner entry point used by both the CLI dispatch path and the unit
/// tests. The launcher and the control requester are injected so tests can
/// spy on what was requested without spawning a real browser.
pub(crate) fn run_with_home(
    gwt_home: &Path,
    target: Option<&Path>,
    out: &mut String,
    launcher: &dyn Fn(&str) -> std::io::Result<()>,
    requester: &ProjectOpenRequester<'_>,
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
    let url = match target {
        None => lock.url.clone(),
        Some(target) => match project_url(&lock, target, requester) {
            Ok(url) => url,
            Err(message) => {
                out.push_str(&format!("gwt open: {message}\n"));
                return Ok(1);
            }
        },
    };
    match launcher(&url) {
        Ok(()) => Ok(0),
        Err(error) => {
            out.push_str(&format!(
                "gwt open: could not launch browser for {url}: {error}\n"
            ));
            Ok(1)
        }
    }
}

/// Ask the running process to open `target`, then build `/p/<key>` on its URL.
fn project_url(
    lock: &TrayLockFile,
    target: &Path,
    requester: &ProjectOpenRequester<'_>,
) -> Result<String, String> {
    let Some(token) = lock
        .control_token
        .as_deref()
        .filter(|token| !token.is_empty())
    else {
        return Err(
            "the running gwt does not accept `gwt open <path>`; restart gwt to update it"
                .to_string(),
        );
    };
    let absolute = std::path::absolute(target)
        .map_err(|error| format!("could not resolve {}: {error}", target.display()))?;
    let response = requester(&lock.url, token, &absolute)?;
    let base = reqwest::Url::parse(&lock.url)
        .map_err(|error| format!("tray lock URL {} is invalid: {error}", lock.url))?;
    base.join(&response.url_path)
        .map(|url| url.to_string())
        .map_err(|error| format!("invalid project URL {}: {error}", response.url_path))
}

/// `POST /internal/projects/open` with the lock's bearer token. The server
/// answers only after the Project has opened (or with its bounded timeout).
fn send_project_open_request(
    server_url: &str,
    token: &str,
    path: &Path,
) -> Result<ProjectOpenControlResponse, String> {
    let endpoint = reqwest::Url::parse(server_url)
        .and_then(|base| base.join(PROJECT_OPEN_CONTROL_PATH))
        .map_err(|error| format!("tray lock URL {server_url} is invalid: {error}"))?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(150))
        .build()
        .map_err(|error| format!("could not build HTTP client: {error}"))?;
    let response = client
        .post(endpoint)
        .bearer_auth(token)
        .json(&ProjectOpenControlRequest {
            path: path.display().to_string(),
        })
        .send()
        .map_err(|error| format!("could not reach the running gwt: {error}"))?;
    let status = response.status();
    if status.is_success() {
        return response
            .json::<ProjectOpenControlResponse>()
            .map_err(|error| format!("invalid response from the running gwt: {error}"));
    }
    let detail = response
        .json::<ProjectOpenControlErrorBody>()
        .map(|body| body.error)
        .unwrap_or_else(|_| "no detail".to_string());
    Err(format!(
        "could not open {} ({}): {detail}",
        path.display(),
        status.as_u16()
    ))
}

/// Spawn the OS-native default browser at `url`. The launcher is
/// detached and a reaper thread waits on the child so repeated
/// invocations do not accumulate zombies on Unix.
fn spawn_default_browser_launcher(url: &str) -> std::io::Result<()> {
    let child = if cfg!(target_os = "macos") {
        gwt_core::process::hidden_command("open").arg(url).spawn()?
    } else if cfg!(target_os = "windows") {
        // The empty "" before the URL is required by `start` so a URL
        // beginning with quoted text is not treated as a window title.
        gwt_core::process::hidden_command("cmd")
            .args(["/C", "start", "", url])
            .spawn()?
    } else {
        gwt_core::process::hidden_command("xdg-open")
            .arg(url)
            .spawn()?
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
        write_lock_with_token(tmp, url, None);
    }

    fn write_lock_with_token(tmp: &TempDir, url: &str, control_token: Option<&str>) {
        let user_id = current_user_id();
        let path = lock_path(tmp.path(), &user_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload = TrayLockFile {
            pid: std::process::id(),
            url: url.to_string(),
            started_at: Utc::now(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            control_token: control_token.map(str::to_string),
        };
        std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
    }

    fn unused_requester(_: &str, _: &str, _: &Path) -> Result<ProjectOpenControlResponse, String> {
        panic!("root open must not send a project-open request");
    }

    fn run_root(
        tmp: &TempDir,
        out: &mut String,
        launcher: &dyn Fn(&str) -> std::io::Result<()>,
    ) -> i32 {
        run_with_home(tmp.path(), None, out, launcher, &unused_requester).expect("run")
    }

    #[test]
    fn parse_args_accepts_zero_arguments() {
        let parsed = parse_args(&[]).expect("empty argv parses");
        assert!(matches!(
            parsed,
            super::super::CliCommand::Open(OpenArgs { path: None })
        ));
    }

    #[test]
    fn parse_args_accepts_one_optional_path() {
        let parsed = parse_args(&["../project".to_string()]).expect("path parses");
        assert!(matches!(
            parsed,
            super::super::CliCommand::Open(OpenArgs { path: Some(path) })
                if path == Path::new("../project")
        ));
    }

    #[test]
    fn parse_args_rejects_unknown_extras() {
        let argv = vec!["--no-such-flag".to_string()];
        let err = parse_args(&argv).expect_err("unknown flag must error");
        assert!(matches!(err, CliParseError::UnknownSubcommand(flag) if flag == "--no-such-flag"));
        let argv = vec!["one".to_string(), "two".to_string()];
        let err = parse_args(&argv).expect_err("a second path must error");
        assert!(matches!(err, CliParseError::UnknownSubcommand(extra) if extra == "two"));
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
        let exit = run_root(&tmp, &mut out, &launcher);
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
        let exit = run_root(&tmp, &mut out, &launcher);
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
        let exit = run_root(&tmp, &mut out, &launcher);
        assert_eq!(exit, 0);
        assert_eq!(
            launched.lock().unwrap().as_slice(),
            ["http://127.0.0.1:55555/".to_string()].as_slice()
        );
        // Successful launch produces no error message.
        assert!(out.is_empty(), "expected silent success but got: {out:?}");
    }

    #[test]
    fn run_with_path_opens_the_project_before_launching_its_url() {
        let tmp = TempDir::new().unwrap();
        write_lock_with_token(&tmp, "http://127.0.0.1:55555/", Some("token-1"));
        let requests: Mutex<Vec<(String, String, PathBuf)>> = Mutex::new(Vec::new());
        let requester = |url: &str, token: &str, path: &Path| {
            requests
                .lock()
                .unwrap()
                .push((url.to_string(), token.to_string(), path.to_path_buf()));
            Ok(ProjectOpenControlResponse {
                project_key: "0123456789abcdef".to_string(),
                url_path: "/p/0123456789abcdef".to_string(),
            })
        };
        let launched: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let launcher = |url: &str| {
            launched.lock().unwrap().push(url.to_string());
            Ok(())
        };
        let mut out = String::new();
        let exit = run_with_home(
            tmp.path(),
            Some(Path::new("relative-project")),
            &mut out,
            &launcher,
            &requester,
        )
        .expect("run");

        assert_eq!(exit, 0, "{out}");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "http://127.0.0.1:55555/");
        assert_eq!(requests[0].1, "token-1");
        assert!(
            requests[0].2.is_absolute(),
            "the server receives an absolute path"
        );
        assert!(requests[0].2.ends_with("relative-project"));
        assert_eq!(
            launched.lock().unwrap().as_slice(),
            ["http://127.0.0.1:55555/p/0123456789abcdef".to_string()].as_slice()
        );
    }

    #[test]
    fn run_with_path_never_launches_when_the_request_fails() {
        let tmp = TempDir::new().unwrap();
        write_lock_with_token(&tmp, "http://127.0.0.1:55555/", Some("token-1"));
        let requester =
            |_: &str, _: &str, _: &Path| -> Result<ProjectOpenControlResponse, String> {
                Err("could not open /x (422): not a project".to_string())
            };
        let launcher = |url: &str| -> std::io::Result<()> {
            panic!("no speculative launch, got {url}");
        };
        let mut out = String::new();
        let exit = run_with_home(
            tmp.path(),
            Some(Path::new("/x")),
            &mut out,
            &launcher,
            &requester,
        )
        .expect("run");
        assert_eq!(exit, 1);
        assert!(out.contains("422"), "{out}");
    }

    #[test]
    fn run_with_path_refuses_a_legacy_lock_without_a_control_token() {
        let tmp = TempDir::new().unwrap();
        write_lock(&tmp, "http://127.0.0.1:55555/");
        let launcher = |url: &str| -> std::io::Result<()> {
            panic!("no speculative launch, got {url}");
        };
        let mut out = String::new();
        let exit = run_with_home(
            tmp.path(),
            Some(Path::new("/x")),
            &mut out,
            &launcher,
            &unused_requester,
        )
        .expect("run");
        assert_eq!(exit, 1);
        assert!(out.contains("restart gwt"), "{out}");
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
        let _home = gwt_core::test_support::ScopedGwtHome::set(tmp.path());

        let mut env = TestEnv::new(std::path::PathBuf::from("cache-root"));
        let mut out = String::new();
        let exit = run(&mut env, OpenArgs::default(), &mut out).unwrap();

        assert_eq!(exit, 1, "isolated home has no tray lock");
    }
}
