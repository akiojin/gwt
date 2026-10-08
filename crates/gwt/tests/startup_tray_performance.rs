//! Issues #4730 / #4803: isolated Windows dev-host startup-load measurements.
//! The ignored load fixtures preserve the original host measurement budgets.
//! Keep their <= 5 s canvas / <= 500 ms response budgets on the Windows dev host.
//! Run either fixture explicitly after building this checkout:
//! cargo test -p gwt --test startup_tray_performance <fixture_name> -- --exact --ignored --test-threads=1 --nocapture
//! PR CI selects only startup_metric_and_native_command_contract with --exact
//! and without --ignored; the 1500-session Git-spawn budget runs as a unit test.
//! Nightly selects startup_tray_under_large_stopped_session_load explicitly.
//! The update-marker fixture remains dev-host-only (Issues #4839 / #4821).
//! This observes the native tray window and sends the real muda WM_COMMAND route;
//! it does NOT prove Explorer icon painting or a physical notification-area click.
//! Command IDs follow the native menu allocation and main's initial creation order.
#![cfg(windows)]

use std::{
    ffi::c_void,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use gwt::{issue_monitor as monitor, pm_registry as pm};
use gwt_agent::{AgentId, AgentStatus, Session};
use gwt_core::process::{hidden_command, scrub_git_env};
use serde_json::{json, Value};

const REPEATS: usize = 5;
const POLL: Duration = Duration::from_millis(100);
const TIMEOUT: Duration = Duration::from_secs(60);
const OPEN: usize = 1001;
const QUIT: usize = 1004;
type Hwnd = *mut c_void;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(callback: unsafe extern "system" fn(Hwnd, isize) -> i32, data: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    fn GetClassNameW(hwnd: Hwnd, name: *mut u16, capacity: i32) -> i32;
    fn PostMessageW(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> i32;
    fn SendMessageTimeoutW(
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;
}

fn tray_window(pid: u32) -> Option<Hwnd> {
    unsafe extern "system" fn find(hwnd: Hwnd, data: isize) -> i32 {
        // EnumWindows invokes this synchronously while the stack tuple is alive.
        let search = unsafe { &mut *(data as *mut (u32, Option<Hwnd>)) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == search.0 {
            let mut name = [0_u16; 64];
            let len = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
            if len > 0 && String::from_utf16_lossy(&name[..len as usize]) == "tray_icon_app" {
                search.1 = Some(hwnd);
                return 0;
            }
        }
        1
    }
    let mut search = (pid, None);
    unsafe { EnumWindows(find, &mut search as *mut _ as isize) };
    search.1
}

fn menu_command(hwnd: Hwnd, id: usize) {
    assert_ne!(
        unsafe { PostMessageW(hwnd, 0x0111, id, 0) },
        0,
        "post native menu command"
    );
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        // Let native Quit finish; never kill a browser process or an entire process tree.
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(self.0.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn git(root: &Path, args: &[&str]) {
    let mut command = hidden_command("git");
    scrub_git_env(&mut command);
    let output = command
        .current_dir(root)
        .args(args)
        .output()
        .expect("fixture git");
    assert!(output.status.success(), "fixture git: {output:?}");
}

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    template: PathBuf,
}

impl Fixture {
    fn new(worktree_count: usize) -> Self {
        let temp = tempfile::tempdir().expect("isolated startup fixture");
        let repo = temp.path().join("repo");
        let template = temp.path().join("session-template");
        for dir in [&repo, &template] {
            fs::create_dir_all(dir).unwrap();
        }
        git(&repo, &["init", "--quiet"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "fixture"],
        );
        // Real worktrees including the main worktree; no production repo is touched.
        let mut worktrees = vec![repo.clone()];
        for index in 1..worktree_count {
            let path = temp.path().join(format!("wt-{index}"));
            let target = path.to_str().unwrap();
            git(
                &repo,
                &["worktree", "add", "--quiet", "--detach", target, "HEAD"],
            );
            worktrees.push(path);
        }
        let old = chrono::Utc::now() - chrono::Duration::days(7);
        for index in 0..1500 {
            let root = &worktrees[index % worktrees.len()];
            let mut session = Session::new(root, "fixture", AgentId::Codex);
            session.status = AgentStatus::Stopped;
            session.restore_window_on_startup = false;
            session.created_at = old;
            session.updated_at = old;
            session.last_activity_at = old;
            session.save(&template).expect("seed stopped Session");
        }
        Self {
            _temp: temp,
            repo,
            template,
        }
    }

    fn home(&self, run: usize) -> PathBuf {
        let home = self._temp.path().join(format!("home-{run}"));
        let sessions = home.join(".gwt/sessions");
        fs::create_dir_all(&sessions).unwrap();
        for entry in fs::read_dir(&self.template).unwrap().flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "toml") {
                let path = sessions.join(entry.file_name());
                fs::copy(entry.path(), &path).unwrap();
                let old = std::time::SystemTime::now() - Duration::from_secs(7 * 24 * 60 * 60);
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(old))
                    .unwrap();
            }
        }
        let _scope = gwt_core::test_support::ScopedGwtHome::set(&home);
        let mut prefs = pm::PmPrefs::default();
        prefs.settings.auto_start = false;
        pm::save_pm_prefs(&pm::pm_prefs_path_for_repo_path(&self.repo), &prefs).unwrap();
        let monitor = monitor::IssueMonitorPrefs {
            enabled: false,
            ..Default::default()
        };
        let prefs_path = monitor::issue_monitor_prefs_path_for_repo_path(&self.repo);
        monitor::save_issue_monitor_prefs(&prefs_path, &monitor).unwrap();
        fs::write(
            home.join(".gwt/config.toml"),
            "[usage]\ncodex_enabled = false\nclaude_account_enabled = false\n",
        )
        .unwrap();
        let state = gwt::PersistedSessionState {
            tabs: vec![gwt::PersistedSessionTabState {
                id: "fixture".into(),
                title: "Fixture".into(),
                project_root: self.repo.clone(),
                kind: gwt::ProjectKind::Git,
            }],
            legacy_active_tab_id: None,
            recent_projects: Vec::new(),
        };
        let path = gwt_core::paths::gwt_session_state_path();
        gwt::save_session_state(&path, &state).unwrap();
        assert_eq!(
            gwt::load_session_state(&path).unwrap().tabs,
            state.tabs,
            "AppRuntime::new must restore the fixture project from the canonical file"
        );
        home
    }

    fn configure(&self, command: &mut Command, home: &Path) {
        scrub_git_env(command);
        for (key, _) in std::env::vars_os() {
            let key_text = key.to_string_lossy();
            if key_text.starts_with("GWT_")
                || key_text == "CODEX_HOME"
                || key_text == "CLAUDE_CONFIG_DIR"
            {
                command.env_remove(key);
            }
        }
        command
            .current_dir(&self.repo)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("CI", "1")
            .env("RUST_LOG", "info")
            .env("APPDATA", home.join("AppData/Roaming"))
            .env("LOCALAPPDATA", home.join("AppData/Local"))
            .env("GWT_DISABLE_BACKGROUND_INDEX", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(Stdio::null());
    }
}

fn logs(dir: &Path) -> String {
    let mut text = String::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            text.push_str(&logs(&path));
        } else {
            text.push_str(&fs::read_to_string(path).unwrap_or_default());
            text.push('\n');
        }
    }
    text
}

fn tray_ready_ms(log: &str) -> Option<f64> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|row| {
            let sample = row.get("startup")?;
            (sample.get("phase")?.as_str()? == "tray_ready")
                .then(|| Some(sample.get("start_ms")?.as_f64()? + row.get("value")?.as_f64()?))
                .flatten()
        })
}

#[test]
fn startup_metric_and_native_command_contract() {
    let mark = r#"{"startup":{"phase":"tray_ready","start_ms":0},"value":1750}"#;
    assert_eq!(tray_ready_ms(mark), Some(1750.0));
    let menu = include_str!("../src/main.rs")
        .rsplit_once("let tray_menu = Menu::new();")
        .unwrap()
        .1;
    let menu = menu.split("let tray_projects").next().unwrap();
    assert_eq!(menu.matches("MenuItem::with_id(").count(), QUIT - OPEN + 1);
    let mut previous = 0;
    for item in [
        "let tray_open",
        "let tray_copy_url =",
        "let tray_about",
        "let tray_quit",
    ] {
        let position = menu.find(item).expect("native menu item construction");
        assert!(position > previous, "native command ID order changed");
        previous = position;
    }
}

#[test]
#[ignore = "real Windows tray, 300 worktrees and 1500 stopped Sessions; run explicitly"]
fn startup_tray_under_large_stopped_session_load() {
    let fixture = Fixture::new(300);
    let control_binary = std::env::var_os("GWT_STARTUP_CONTROL_BINARY");
    let is_control = control_binary.is_some();
    let binary = control_binary.unwrap_or_else(|| env!("CARGO_BIN_EXE_gwt").into());
    startup_metric_and_native_command_contract();
    let mut samples = Vec::new();
    let mut dispatch_observation_complete = false;
    let mut project_restore_completed = false;
    let mut max_logged_dispatch_ms: f64 = 0.0;
    let mut probe_timeouts = 0;
    let mut diagnostic_logs = Vec::new();
    for run in 0..REPEATS {
        let home = fixture.home(run);
        let capture = home.join("stderr.log");
        let mut command = hidden_command(&binary);
        fixture.configure(&mut command, &home);
        command
            .stdout(Stdio::null())
            .stderr(fs::File::create(&capture).unwrap());
        let started = Instant::now();
        let mut child = Running(command.spawn().expect("spawn checkout gwt"));
        let mut hwnd = None;
        while started.elapsed() < TIMEOUT {
            hwnd = tray_window(child.0.id());
            if hwnd.is_some() || child.0.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(POLL);
        }
        let native_window_ms = started.elapsed().as_secs_f64() * 1000.0;
        let log_dir = home.join(".gwt/logs");
        let mut text = logs(&log_dir);
        let mut ready = tray_ready_ms(&text);
        // Wait for buffered instrumentation, but never mistake a baseline without
        // TrayReady for a passing sample or send its already-ready Open action.
        let flush = Instant::now();
        while hwnd.is_some() && ready.is_none() && flush.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(POLL);
            text = logs(&log_dir);
            ready = tray_ready_ms(&text);
        }
        let mut click_ms = None;
        if let Some(hwnd) = hwnd {
            if ready.is_some() && !is_control {
                let click = Instant::now();
                menu_command(hwnd, OPEN);
                while click.elapsed() < Duration::from_secs(2) {
                    text = logs(&log_dir);
                    if text.contains("tray request accepted during startup")
                        || text.contains("tray Open browser launch requested")
                    {
                        click_ms = Some(click.elapsed().as_secs_f64() * 1000.0);
                        break;
                    }
                    std::thread::sleep(POLL);
                }
            }
            if run == REPEATS - 1 && ready.is_some() && !is_control {
                // Observe the whole first ten minutes, not just immediate tray readiness.
                loop {
                    if child.0.try_wait().unwrap().is_some() {
                        break;
                    }
                    // WM_NULL is non-mutating; a 500ms timeout also catches a dispatch
                    // that never returns and thus never emits its final duration log.
                    let mut result = 0;
                    if unsafe { SendMessageTimeoutW(hwnd, 0, 0, 0, 3, 500, &mut result) } == 0 {
                        probe_timeouts += 1;
                        break;
                    }
                    if started.elapsed() >= Duration::from_secs(600) {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                dispatch_observation_complete = started.elapsed() >= Duration::from_secs(600)
                    && child.0.try_wait().unwrap().is_none();
            }
        }
        // Include final dispatch completion logs before shutdown, and diagnostics
        // from short/failed trials as well as the ten-minute observation.
        std::thread::sleep(POLL);
        text = logs(&log_dir);
        let project_logs = {
            let _scope = gwt_core::test_support::ScopedGwtHome::set(&home);
            gwt_core::paths::gwt_project_dir_for_repo_path(&fixture.repo).join("logs")
        };
        text.push_str(&logs(&project_logs));
        let mut phases = Vec::new();
        for row in text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        {
            if run == REPEATS - 1 && row["startup"]["phase"] == "project_state_load" {
                project_restore_completed = true;
            }
            let mut diagnostic = row["level"].as_str() == Some("ERROR");
            if row["target"] == "gwt.frontend.timing" {
                if let Some(elapsed) = row["fields"]["elapsed_ms"].as_f64() {
                    max_logged_dispatch_ms = max_logged_dispatch_ms.max(elapsed);
                    diagnostic |= elapsed > 500.0;
                }
            }
            if row["startup"].is_object() && phases.len() < 20 {
                phases.push(json!({"phase":row["startup"]["phase"],
                    "start_ms":row["startup"]["start_ms"], "duration_ms":row["value"]}));
            }
            if diagnostic && diagnostic_logs.len() < 5 {
                diagnostic_logs.push(row);
            }
        }
        let failed = hwnd.is_none() || ready.is_none() || click_ms.is_none();
        let stderr = failed.then(|| {
            fs::read_to_string(&capture)
                .unwrap_or_default()
                .lines()
                .take(20)
                .collect::<Vec<_>>()
                .join("\n")
        });
        if let Some(hwnd) = hwnd {
            if child.0.try_wait().unwrap().is_none() {
                menu_command(hwnd, QUIT);
            }
        }
        samples.push(
            json!({"run":run, "native_window_ms":hwnd.map(|_| native_window_ms),
            "native_window_found":hwnd.is_some(), "tray_ready_ms":ready, "click_ack_ms":click_ms,
            "startup_phases":phases, "failure_stderr":stderr}),
        );
        drop(child);
        if ready.is_none() && !is_control {
            break;
        }
    }
    let percentile = |field: &str| {
        let mut values = samples
            .iter()
            .filter_map(|row| row[field].as_f64())
            .collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        (values.len() == REPEATS).then(|| values[values.len() - 1]) // nearest-rank p95 of five
    };
    let ready_p95 = percentile("tray_ready_ms");
    let native_p95 = percentile("native_window_ms");
    let click_p95 = percentile("click_ack_ms");
    let passed = ready_p95.is_some_and(|ms| ms <= 2000.0)
        && native_p95.is_some_and(|ms| ms <= 2000.0)
        && click_p95.is_some_and(|ms| ms <= 500.0)
        && dispatch_observation_complete
        && project_restore_completed
        && probe_timeouts == 0
        && max_logged_dispatch_ms <= 500.0;
    let report = json!({"passed":passed, "control":is_control, "binary":Path::new(&binary).display().to_string(), "scope":"native menu dispatch; not Explorer painting",
        "sessions":1500, "worktrees":300, "required_repeats":REPEATS, "samples":samples,
        "tray_ready_p95_ms":ready_p95, "native_window_p95_ms":native_p95, "click_ack_p95_ms":click_p95,
        "project_restore_completed":project_restore_completed,
        "dispatch_observation_seconds":600, "dispatch_observation_complete":dispatch_observation_complete,
        "max_logged_dispatch_ms":max_logged_dispatch_ms,
        "native_response_probe_timeouts":probe_timeouts, "diagnostic_logs":diagnostic_logs,
        "dispatch_measurement":"frontend timing target only; durations below the 100ms warning threshold are not logged",
        "missing_measurements":"null means unmeasured/RED, never zero"});
    let out =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/startup-tray-performance.json");
    fs::create_dir_all(out.parent().unwrap()).unwrap();
    fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    eprintln!("{report}");
    assert!(passed, "startup tray budget failed or unmeasured");
}

/// Issue #4803: the update marker forces the historical Session path that
/// ordinary cold-start coverage does not exercise.
#[test]
#[ignore = "real Windows startup with an update marker and 1500 Sessions"]
fn startup_update_resume_under_large_session_load() {
    // More worktrees than the Git budget proves a per-worktree Git cache
    // alone is insufficient, without repeating the separate 300-worktree suite.
    let fixture = Fixture::new(6);
    git(
        &fixture.repo,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/startup-fixture.git",
        ],
    );
    let home = fixture.home(0);
    {
        let _scope = gwt_core::test_support::ScopedGwtHome::set(&home);
        gwt_core::update::persist_update_resume_marker(&gwt_core::update::UpdateResumeMarker {
            from_version: "fixture".into(),
            to_version: env!("CARGO_PKG_VERSION").into(),
            started_at: chrono::Utc::now().to_rfc3339(),
            restart_args: Vec::new(),
            projects: vec![gwt_core::update::UpdateResumeProject {
                hash: gwt_core::paths::project_scope_hash(&fixture.repo).to_string(),
                update_drain: false,
            }],
            attempt: 1,
        })
        .unwrap();
    }
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwt"));
    fixture.configure(&mut command, &home);
    let url_file = home.join("browser-url.txt");
    command
        .arg("--no-open")
        .env("GWT_BROWSER_URL_FILE", &url_file)
        .stdout(Stdio::null())
        .stderr(fs::File::create(home.join("stderr.log")).unwrap());
    let mut child = Running(command.spawn().expect("spawn checkout gwt"));
    let started = Instant::now();
    let browser_binary = std::env::var_os("GWT_TEST_CHROMIUM")
        .map(PathBuf::from)
        .or_else(|| {
            ["ProgramFiles(x86)", "ProgramFiles"]
                .into_iter()
                .find_map(|key| {
                    let path = PathBuf::from(std::env::var_os(key)?)
                        .join("Microsoft/Edge/Application/msedge.exe");
                    path.is_file().then_some(path)
                })
        })
        .expect("set GWT_TEST_CHROMIUM or install Chromium-based Microsoft Edge");
    let mut browser = None;
    let mut browser_url = None;
    let mut browser_started_ms = None;
    let mut canvas = None;
    let mut drain = None;
    let mut parsed_sessions = None;
    let mut common_dir_spawns = std::collections::HashSet::new();
    let mut process_logging_seen = false;
    let mut phases = Vec::new();
    while started.elapsed() < TIMEOUT && child.0.try_wait().unwrap().is_none() {
        if browser.is_none() {
            if let Ok(url) = fs::read_to_string(&url_file) {
                if !url.trim().is_empty() {
                    let project_url = format!(
                        "{}p/{}",
                        url.trim().trim_end_matches('/').to_owned() + "/",
                        gwt_core::paths::project_scope_hash(&fixture.repo)
                    );
                    browser_url = Some(project_url.clone());
                    browser_started_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                    browser = Some(Running(
                        hidden_command(&browser_binary)
                            .arg(format!(
                                "--user-data-dir={}",
                                home.join("chromium-profile").display()
                            ))
                            .args([
                                "--no-first-run",
                                "--no-default-browser-check",
                                "--disable-background-networking",
                                // Keep the headed measurement rendering when another
                                // window occludes it, as browser test runners do.
                                "--disable-background-timer-throttling",
                                "--disable-backgrounding-occluded-windows",
                                "--disable-renderer-backgrounding",
                                "--no-sandbox",
                                "--remote-debugging-port=0",
                            ])
                            .arg(format!("--app={project_url}"))
                            .stdout(Stdio::null())
                            .stderr(fs::File::create(home.join("chromium-stderr.log")).unwrap())
                            .spawn()
                            .expect("start isolated headed Chromium"),
                    ));
                }
            }
        }
        let mut text = logs(&home.join(".gwt/logs"));
        let project_logs = {
            let _scope = gwt_core::test_support::ScopedGwtHome::set(&home);
            gwt_core::paths::gwt_project_dir_for_repo_path(&fixture.repo).join("logs")
        };
        text.push_str(&logs(&project_logs));
        phases.clear();
        common_dir_spawns.clear();
        for row in text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        {
            if row["target"] == "gwt.process.summary" && row["fields"]["phase"] == "start" {
                process_logging_seen = true;
                if row["fields"]["label"].as_str().is_some_and(|label| {
                    label.contains("rev-parse") && label.contains("--git-common-dir")
                }) {
                    common_dir_spawns.insert(format!(
                        "{}:{}:{}",
                        row["timestamp"], row["fields"]["spawn_id"], row["fields"]["label"]
                    ));
                }
            }
            match row["startup"]["phase"].as_str() {
                Some("canvas_ready") => canvas = row["value"].as_f64(),
                Some("restore_drain") => drain = row["value"].as_f64(),
                Some("session_load") => parsed_sessions = row["startup"]["count"].as_u64(),
                _ => {}
            }
            if row["startup"].is_object() || row["target"] == "gwt.process.summary" {
                phases.push(row);
            }
        }
        if canvas.is_some() && drain.is_some() {
            break;
        }
        std::thread::sleep(POLL);
    }
    let passed = parsed_sessions == Some(1500)
        && process_logging_seen
        && common_dir_spawns.len() <= 5
        && canvas.is_some_and(|ms| ms <= 5000.0)
        && drain.is_some_and(|ms| ms <= 500.0);
    let browser_exit = browser
        .as_mut()
        .and_then(|browser| browser.0.try_wait().ok().flatten());
    let report = json!({"passed":passed, "sessions":1500, "worktrees":6,
        "update_resume_marker":true, "parsed_sessions":parsed_sessions, "canvas_ready_ms":canvas,
        "common_dir_git_spawns":common_dir_spawns.len(), "process_logging_seen":process_logging_seen,
        "restore_drain_ms":drain, "perf_log_excerpt":phases,
        "browser_started":browser.is_some(), "browser_binary":browser_binary,
        "browser_url":browser_url, "browser_started_ms":browser_started_ms,
        "browser_exit":browser_exit.map(|status|status.to_string()),
        "browser_stderr":fs::read_to_string(home.join("chromium-stderr.log")).unwrap_or_default(),
        "fixture_home":home,
        "failure_stderr":(!passed).then(||fs::read_to_string(home.join("stderr.log")).unwrap_or_default())});
    let out =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/startup-update-performance.json");
    fs::write(out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    eprintln!("{report}");
    if let Some(hwnd) = tray_window(child.0.id()) {
        menu_command(hwnd, QUIT);
    }
    drop(child);
    drop(browser);
    if !passed {
        let _ = fixture._temp.keep();
    }
    assert!(passed, "update startup budget failed or unmeasured");
}
