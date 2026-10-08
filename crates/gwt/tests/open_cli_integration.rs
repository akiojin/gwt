//! Issue #4538 AC-4 / AC-5: `gwt open [path]` against a running isolated
//! tray-resident `gwt`.
//!
//! The browser launcher is the OS `open` / `xdg-open` found on `PATH`; a fake
//! one records the URL it was asked to open, so the test observes exactly
//! what a user's browser would receive and that failures launch nothing.
#![cfg(unix)]

use gwt_core::process::{hidden_command, scrub_git_env};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Output, Stdio},
    time::{Duration, Instant},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

struct Fixture {
    temp: tempfile::TempDir,
    home: PathBuf,
    fake_bin: PathBuf,
    opened: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let home = temp.path().join("home");
        let fake_bin = temp.path().join("bin");
        let opened = temp.path().join("opened.log");
        std::fs::create_dir_all(home.join(".gwt")).expect("isolated gwt home");
        std::fs::write(
            home.join(".gwt/config.toml"),
            "[board]\noauth_redirect_port = 0\n",
        )
        .expect("seed config");
        std::fs::create_dir_all(&fake_bin).expect("fake bin");
        for launcher in ["open", "xdg-open"] {
            let script = fake_bin.join(launcher);
            std::fs::write(
                &script,
                format!("#!/bin/sh\necho \"$@\" >> '{}'\n", opened.display()),
            )
            .expect("fake launcher");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("launcher mode");
        }
        Self {
            temp,
            home,
            fake_bin,
            opened,
        }
    }

    fn isolate(&self, command: &mut std::process::Command) {
        scrub_git_env(command);
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_CACHE_HOME", self.home.join("xdg-cache"))
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .env("CI", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .env_remove("GWT_SESSION_ID")
            .env_remove("GWT_PROJECT_ROOT")
            .env_remove("GWT_PROJECT_ROOT_HASH")
            .env_remove("GWT_WORKTREE_HASH")
            .env_remove("GWT_WORKSPACE_ID")
            .env_remove("GWT_RUNTIME_DIR")
            .env_remove("GWT_FORCE_NEW_INSTANCE");
    }

    fn start_server(&self) -> Server {
        let url_path = self.temp.path().join("browser-url.txt");
        let workspace = self.temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let mut command = hidden_command(env!("CARGO_BIN_EXE_gwt"));
        self.isolate(&mut command);
        command
            .args(["--no-tray", "--no-open"])
            .current_dir(&workspace)
            .env("GWT_BROWSER_URL_FILE", &url_path)
            .stdout(Stdio::from(
                File::create(self.temp.path().join("stdout.log")).expect("stdout"),
            ))
            .stderr(Stdio::from(
                File::create(self.temp.path().join("stderr.log")).expect("stderr"),
            ));
        let child = command.spawn().expect("spawn isolated gwt");
        let server = Server::wait_until_ready(child, &url_path);
        // The browser URL handoff precedes the tray lock URL update; `gwt
        // open` discovers the server through the lock.
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while self.lock_payload().0["url"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
        {
            assert!(
                Instant::now() < deadline,
                "tray lock never published its URL"
            );
            std::thread::sleep(POLL_INTERVAL);
        }
        server
    }

    fn gwt_open(&self, args: &[&Path]) -> Output {
        let mut command = hidden_command(env!("CARGO_BIN_EXE_gwt"));
        self.isolate(&mut command);
        let path = format!(
            "{}:{}",
            self.fake_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        command.arg("open").args(args).env("PATH", path);
        command.output().expect("run gwt open")
    }

    /// URLs the fake browser launcher received, waiting briefly for the
    /// detached launcher to write.
    fn opened_urls(&self, expected: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let urls: Vec<String> = std::fs::read_to_string(&self.opened)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
            if urls.len() >= expected || Instant::now() >= deadline {
                return urls;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn lock_payload(&self) -> (serde_json::Value, u32) {
        let run = self.home.join(".gwt/run");
        let lock = std::fs::read_dir(&run)
            .expect("run dir")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension().is_some_and(|ext| ext == "lock")
                    && path.file_name().is_some_and(|name| {
                        let name = name.to_string_lossy();
                        name.starts_with("tray-") && !name.contains("-forced-")
                    })
            })
            .expect("tray lock");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&lock)
            .expect("lock metadata")
            .permissions()
            .mode();
        let payload = serde_json::from_str(&std::fs::read_to_string(&lock).expect("read lock"))
            .expect("lock json");
        (payload, mode & 0o777)
    }
}

struct Server {
    child: Child,
    url: String,
}

impl Server {
    fn wait_until_ready(mut child: Child, url_path: &Path) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .expect("client");
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait().expect("inspect child") {
                panic!("gwt exited before readiness ({status})");
            }
            if let Ok(url) = std::fs::read_to_string(url_path) {
                let url = url.trim().to_string();
                if !url.is_empty()
                    && client
                        .get(format!("{url}healthz"))
                        .send()
                        .is_ok_and(|response| response.status().is_success())
                {
                    return Self { child, url };
                }
            }
            assert!(Instant::now() < deadline, "gwt did not become ready");
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        // SAFETY: the PID belongs to this live child; SIGTERM is the
        // supported graceful shutdown.
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn post_open(url: &str, authorization: Option<&str>, path: &Path) -> reqwest::blocking::Response {
    let mut request = reqwest::blocking::Client::new()
        .post(format!("{url}internal/projects/open"))
        .json(&serde_json::json!({ "path": path.display().to_string() }));
    if let Some(authorization) = authorization {
        request = request.header("Authorization", authorization);
    }
    request.send().expect("control request")
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "tao requires DISPLAY/WAYLAND; run with xvfb-run -a"
)]
fn gwt_open_path_opens_the_project_then_launches_its_url() {
    let fixture = Fixture::new();
    let server = fixture.start_server();
    let project = fixture.temp.path().join("unopened-project");
    std::fs::create_dir_all(&project).expect("project dir");
    let status = hidden_command("git")
        .args(["init", "-q"])
        .current_dir(&project)
        .status()
        .expect("git init");
    assert!(status.success());

    // The lock publishes the control token to this OS user only.
    let (lock, mode) = fixture.lock_payload();
    assert_eq!(mode, 0o600);
    let token = lock["control_token"].as_str().expect("control token");
    assert_eq!(token.len(), 64);

    // Unauthorized requests are refused and open nothing.
    assert_eq!(post_open(&server.url, None, &project).status(), 401);
    assert_eq!(
        post_open(&server.url, Some("Bearer wrong-token"), &project).status(),
        401
    );

    // Argument-free `gwt open` still opens the Hub.
    let root = fixture.gwt_open(&[]);
    assert!(root.status.success(), "{root:?}");
    assert_eq!(fixture.opened_urls(1), vec![server.url.clone()]);

    // `gwt open <path>` opens the unopened Project, then its per-project URL.
    let opened = fixture.gwt_open(&[&project]);
    assert!(opened.status.success(), "{opened:?}");
    let urls = fixture.opened_urls(2);
    assert_eq!(urls.len(), 2, "{urls:?}");
    let project_url = &urls[1];
    let key = project_url
        .strip_prefix(&format!("{}p/", server.url))
        .expect("per-project URL on the running server");
    assert!(
        key.len() == 16
            && key
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
        "{project_url}"
    );
    assert!(!project_url.contains(&project.display().to_string()));
    let page = reqwest::blocking::get(project_url).expect("project page");
    assert_eq!(page.status(), 200);

    // The same path resolves to the same Project key.
    let again = post_open(&server.url, Some(&format!("Bearer {token}")), &project);
    assert_eq!(again.status(), 200);
    let body: serde_json::Value = again.json().expect("json");
    assert_eq!(body["project_key"], key);
    assert_eq!(body["url_path"], format!("/p/{key}"));

    // A path that cannot be opened fails without a speculative launch.
    let missing = fixture.gwt_open(&[&fixture.temp.path().join("missing")]);
    assert_eq!(missing.status.code(), Some(1), "{missing:?}");
    assert!(String::from_utf8_lossy(&missing.stdout)
        .to_string()
        .chars()
        .chain(String::from_utf8_lossy(&missing.stderr).chars())
        .collect::<String>()
        .contains("422"));
    assert_eq!(fixture.opened_urls(3).len(), 2, "no launch after a failure");
}
