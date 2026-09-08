use gwt_core::process::{hidden_command, scrub_git_env};
use std::{
    fs::File,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

/// These tests reserve an ephemeral port, release it, and then assert that a
/// freshly started `gwt` binds *that* port — `occupied_saved_port_...` and
/// `forced_secondary_...` and `explicit_ports_are_transient` all do it, and
/// `implicit_start_...` restarts expecting the first run's port back. A
/// concurrent test's `gwt` can take the released port in the gap, so the
/// serialization is a correctness requirement, not a convenience (Issue #4134
/// AC-4). It lives here rather than in the CI step's `--test-threads=1`, which
/// is therefore belt-and-braces: this lock is what actually holds.
static PROCESS_TEST_LOCK: Mutex<()> = Mutex::new(());
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Issue #4134 AC-4: this file costs 3.9s for nine spawn/stop cycles on macOS
/// and 226s under Xvfb on the Linux CI runner — 42% of that job's test time
/// for 0.05% of its tests. The 58x gap is not the serialization above (which
/// costs the same on both), so it is inside a single `gwt` lifecycle. These
/// lines attribute it: the CI step runs with `--nocapture` so a run says
/// whether the wait is startup or shutdown, and whether shutdown ended on
/// SIGTERM or on the hard kill after `SHUTDOWN_TIMEOUT`.
///
/// The first instrumented run (PR #4144, run 34190754660) answered half of it:
/// shutdown costs 0.03s, and startup costs 25.10s on eight of the nine spawns
/// — a number that does not vary to two decimal places, so it is a timer
/// rather than work. `gwt` builds its `tao::EventLoop`, and with it GTK, long
/// before the embedded server publishes a URL, and 25 seconds is GDBus's
/// default call timeout: under bare `xvfb-run` there is a display but no
/// session bus, so the first GTK call that needs one waits out the timeout.
///
/// Opting the child out of GTK's ATK bridge (`NO_AT_BRIDGE=1`,
/// `GTK_A11Y=none`) was tried first and did **not** move the number (run
/// 34192244659), so the bus consumer is not the accessibility bridge. The CI
/// step now supplies a real session bus with `dbus-run-session` instead, which
/// covers every consumer rather than one. These measurement lines stay: if a
/// run still reports ~25s, that explanation is wrong too and the log says so.
fn report_phase(phase: &str, detail: &str, elapsed: Duration) {
    eprintln!(
        "stable_server_port: {phase} {detail} in {:.2}s",
        elapsed.as_secs_f64()
    );
}

struct StablePortFixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    workspace: PathBuf,
    config_path: PathBuf,
    index_fixture: PathBuf,
    run_sequence: usize,
}

impl StablePortFixture {
    fn new(config: &str) -> Self {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let home = temp.path().join("home");
        let workspace = temp.path().join("workspace");
        let gwt_home = home.join(".gwt");
        let config_path = gwt_home.join("config.toml");
        let index_fixture = temp.path().join("index-status.json");
        std::fs::create_dir_all(&gwt_home).expect("create isolated gwt home");
        std::fs::create_dir_all(&workspace).expect("create isolated workspace");
        std::fs::write(&config_path, config).expect("seed config");
        std::fs::write(
            &index_fixture,
            r#"{"state":"ready","detail":"stable-port integration fixture"}"#,
        )
        .expect("write index fixture");
        Self {
            _temp: temp,
            home,
            workspace,
            config_path,
            index_fixture,
            run_sequence: 0,
        }
    }

    fn spawn(&mut self, extra_args: &[String], forced_secondary: bool) -> RunningGwt {
        self.run_sequence += 1;
        let url_path = self
            ._temp
            .path()
            .join(format!("browser-url-{}.txt", self.run_sequence));
        let stdout_path = self
            ._temp
            .path()
            .join(format!("stdout-{}.log", self.run_sequence));
        let stderr_path = self
            ._temp
            .path()
            .join(format!("stderr-{}.log", self.run_sequence));
        let stdout = File::create(&stdout_path).expect("create stdout capture");
        let stderr = File::create(&stderr_path).expect("create stderr capture");

        let mut command = hidden_command(env!("CARGO_BIN_EXE_gwt"));
        scrub_git_env(&mut command);
        command
            .args(["--no-tray", "--no-open"])
            .args(extra_args)
            .current_dir(&self.workspace)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_CACHE_HOME", self.home.join("xdg-cache"))
            .env("XDG_DATA_HOME", self.home.join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .env("CI", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GWT_BROWSER_URL_FILE", &url_path)
            .env("GWT_INDEX_TEST_FIXTURE", &self.index_fixture)
            .env_remove("GWT_SESSION_ID")
            .env_remove("GWT_PROJECT_ROOT")
            .env_remove("GWT_PROJECT_ROOT_HASH")
            .env_remove("GWT_WORKTREE_HASH")
            .env_remove("GWT_WORKSPACE_ID")
            .env_remove("GWT_RUNTIME_DIR")
            .env_remove("GWT_FORCE_NEW_INSTANCE")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        if forced_secondary {
            command.env("GWT_FORCE_NEW_INSTANCE", "1");
        }
        let child = command.spawn().expect("spawn isolated gwt");
        RunningGwt::wait_until_ready(child, url_path, stdout_path, stderr_path)
    }

    fn config_bytes(&self) -> Vec<u8> {
        std::fs::read(&self.config_path).expect("read config bytes")
    }
}

struct RunningGwt {
    child: Option<Child>,
    url: String,
    stderr_path: PathBuf,
}

impl RunningGwt {
    fn wait_until_ready(
        child: Child,
        url_path: PathBuf,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
    ) -> Self {
        let mut running = Self {
            child: Some(child),
            url: String::new(),
            stderr_path,
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .expect("health client");
        let started = Instant::now();
        let deadline = started + STARTUP_TIMEOUT;
        loop {
            let status = running
                .child
                .as_mut()
                .expect("running child")
                .try_wait()
                .expect("inspect child");
            if let Some(status) = status {
                panic!(
                    "gwt exited before readiness ({status})\nstdout:\n{}\nstderr:\n{}",
                    read_capture(&stdout_path),
                    running.stderr(),
                );
            }
            if let Ok(raw_url) = std::fs::read_to_string(&url_path) {
                let url = raw_url.trim();
                if !url.is_empty()
                    && client
                        .get(format!("{url}healthz"))
                        .send()
                        .is_ok_and(|response| response.status().is_success())
                {
                    running.url = url.to_string();
                    report_phase("startup", "served healthz", started.elapsed());
                    return running;
                }
            }
            assert!(
                Instant::now() < deadline,
                "gwt did not become ready within {STARTUP_TIMEOUT:?}\nstdout:\n{}\nstderr:\n{}",
                read_capture(&stdout_path),
                running.stderr(),
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn port(&self) -> u16 {
        port_from_url(&self.url)
    }

    fn stderr(&self) -> String {
        read_capture(&self.stderr_path)
    }

    fn stop(mut self) {
        if let Some(mut child) = self.child.take() {
            terminate_child(&mut child);
        }
    }
}

impl Drop for RunningGwt {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            terminate_child(&mut child);
        }
    }
}

fn read_capture(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| format!("<unreadable: {error}>"))
}

fn port_from_url(url: &str) -> u16 {
    url.trim_end_matches('/')
        .rsplit(':')
        .next()
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("browser URL must include a port: {url}"))
}

fn reserve_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve port");
    let port = listener.local_addr().expect("reserved address").port();
    (listener, port)
}

fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let started = Instant::now();
    #[cfg(unix)]
    {
        // SAFETY: the PID comes from this live Child and SIGTERM is the
        // application's supported graceful-shutdown path.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
    }
    #[cfg(windows)]
    {
        let _ = child.kill();
    }

    let deadline = started + SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            report_phase("shutdown", "exited on SIGTERM", started.elapsed());
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
    report_phase(
        "shutdown",
        "ignored SIGTERM and needed the hard kill",
        started.elapsed(),
    );
}

fn base_config(extra: &str) -> String {
    format!(
        "# stable-port sentinel\nfuture_unknown_key = \"keep\"\n{extra}\n[board]\noauth_redirect_port = 0\n"
    )
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "tao requires DISPLAY/WAYLAND; run with xvfb-run -a"
)]
fn implicit_start_persists_actual_port_and_restart_reuses_it() {
    let _guard = PROCESS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut fixture = StablePortFixture::new(&base_config(""));

    let first = fixture.spawn(&[], false);
    let first_port = first.port();
    let first_settings = gwt_config::Settings::load_from_path(&fixture.config_path)
        .expect("load first-run settings");
    assert_eq!(
        first_settings.server.embedded_port.map(|port| port.get()),
        Some(first_port),
        "the URL handoff must happen only after the actual implicit port is persisted"
    );
    first.stop();
    let first_config = fixture.config_bytes();

    let second = fixture.spawn(&[], false);
    assert_eq!(second.port(), first_port);
    assert_eq!(fixture.config_bytes(), first_config);
    second.stop();
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "tao requires DISPLAY/WAYLAND; run with xvfb-run -a"
)]
fn occupied_saved_port_falls_back_warns_and_rewrites() {
    let _guard = PROCESS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (_occupied, saved_port) = reserve_port();
    let config = base_config(&format!("\n[server]\nembedded_port = {saved_port}\n"));
    let mut fixture = StablePortFixture::new(&config);

    let running = fixture.spawn(&[], false);
    let actual_port = running.port();
    assert_ne!(actual_port, saved_port);
    let settings =
        gwt_config::Settings::load_from_path(&fixture.config_path).expect("load fallback settings");
    assert_eq!(
        settings.server.embedded_port.map(|port| port.get()),
        Some(actual_port)
    );
    let stderr = running.stderr();
    assert!(
        stderr.contains("saved embedded server port")
            && stderr.contains(&saved_port.to_string())
            && stderr.contains(&actual_port.to_string())
            && stderr.contains("updating config"),
        "fallback warning must identify the old and replacement ports: {stderr}"
    );
    running.stop();
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "tao requires DISPLAY/WAYLAND; run with xvfb-run -a"
)]
fn explicit_ports_are_transient() {
    let _guard = PROCESS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (_saved_occupied, saved_port) = reserve_port();
    let (explicit_reservation, explicit_port) = reserve_port();
    let config = base_config(&format!("\n[server]\nembedded_port = {saved_port}\n"));
    let mut fixture = StablePortFixture::new(&config);
    let original = fixture.config_bytes();
    drop(explicit_reservation);

    let explicit = fixture.spawn(&["--port".to_string(), explicit_port.to_string()], false);
    assert_eq!(explicit.port(), explicit_port);
    assert_eq!(fixture.config_bytes(), original);
    explicit.stop();

    let ephemeral = fixture.spawn(&["--port".to_string(), "0".to_string()], false);
    assert_ne!(ephemeral.port(), saved_port);
    assert_eq!(fixture.config_bytes(), original);
    ephemeral.stop();

    let (explicit_reservation, explicit_port) = reserve_port();
    let mut no_saved_port_fixture = StablePortFixture::new(&base_config(""));
    let no_saved_port_original = no_saved_port_fixture.config_bytes();
    drop(explicit_reservation);

    let explicit_without_saved =
        no_saved_port_fixture.spawn(&["--port".to_string(), explicit_port.to_string()], false);
    assert_eq!(explicit_without_saved.port(), explicit_port);
    assert_eq!(
        no_saved_port_fixture.config_bytes(),
        no_saved_port_original,
        "an explicit non-zero port must not seed an absent persisted port"
    );
    explicit_without_saved.stop();

    let ephemeral_without_saved =
        no_saved_port_fixture.spawn(&["--port".to_string(), "0".to_string()], false);
    assert_eq!(
        no_saved_port_fixture.config_bytes(),
        no_saved_port_original,
        "explicit --port 0 must not seed an absent persisted port"
    );
    ephemeral_without_saved.stop();
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "tao requires DISPLAY/WAYLAND; run with xvfb-run -a"
)]
fn forced_secondary_is_transient_and_preserves_primary_config() {
    let _guard = PROCESS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (reservation, saved_port) = reserve_port();
    let config = base_config(&format!("\n[server]\nembedded_port = {saved_port}\n"));
    let mut fixture = StablePortFixture::new(&config);
    drop(reservation);

    let primary = fixture.spawn(&[], false);
    assert_eq!(primary.port(), saved_port);
    let primary_config = fixture.config_bytes();

    let secondary = fixture.spawn(&[], true);
    assert_ne!(secondary.port(), primary.port());
    assert_eq!(fixture.config_bytes(), primary_config);
    secondary.stop();
    primary.stop();
}
