//! Issue #4535 AC-2 / AC-4 / AC-5 / AC-6 — what a restart does to a legacy
//! session on disk, and what it must not do to the host.
//!
//! The pipeline tests drive the real `~/.gwt` files through the exact
//! persistence entry points `AppRuntime::new` uses, so the collapse, the
//! dropped legacy key and the dormant agent panes are fixed against the files
//! themselves rather than against an in-memory fixture.
//!
//! [`front_door_startup_opens_no_browser`] spawns the real binary and is
//! `#[ignore]`d for the same reason `stable_server_port` is: it needs a display
//! for the `tao` event loop. CI runs it in the `dbus-run-session -- xvfb-run`
//! step alongside that suite.

use gwt::{
    collapse_duplicate_session_tabs, load_restored_workspace_state, load_session_state,
    save_session_state, workspace_state_path, PersistedSessionTabState, ProjectKind, WindowPreset,
    WindowState,
};
use gwt_core::process::{hidden_command, scrub_git_env};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);

/// An isolated `HOME` seeded with a legacy session: three tabs over two
/// distinct repositories, the middle one a second worktree of the first, and a
/// pre-SPEC-#3287 `active_tab_id` naming it.
struct LegacySessionFixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    workspace_home: PathBuf,
    other_home: PathBuf,
}

impl LegacySessionFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(home.join(".gwt")).expect("create isolated gwt home");

        // The duplicate is a *real* alias: one repository seen through its main
        // worktree and through a linked worktree, which is exactly what
        // resolves to one ProjectKey (Issue #4535 AC-1).
        let workspace_home = temp.path().join("projects").join("gwt");
        let other_home = temp.path().join("projects").join("other");
        init_repo(&workspace_home, "https://github.com/example/gwt.git");
        add_worktree(&workspace_home, "develop", &workspace_home.join("develop"));
        init_repo(&other_home, "https://github.com/example/other.git");

        Self {
            _temp: temp,
            home,
            workspace_home,
            other_home,
        }
    }

    fn session_path(&self) -> PathBuf {
        self.home.join(".gwt").join("session-state.json")
    }

    /// The duplicated root and its worktree alias, plus an unrelated project.
    fn tabs(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("project-1", self.workspace_home.clone()),
            ("project-2", self.workspace_home.join("develop")),
            ("project-3", self.other_home.clone()),
        ]
    }

    fn seed_legacy_session(&self, active_tab_id: &str) {
        let tabs = self
            .tabs()
            .into_iter()
            .map(|(id, project_root)| {
                format!(
                    r#"{{"id":{id},"title":{id},"project_root":{root},"kind":"git"}}"#,
                    id = serde_json::to_string(id).expect("tab id json"),
                    root = serde_json::to_string(&project_root).expect("project root json"),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        std::fs::write(
            self.session_path(),
            format!(
                r#"{{"tabs":[{tabs}],"active_tab_id":{active},"recent_projects":[]}}"#,
                active = serde_json::to_string(active_tab_id).expect("active id json"),
            ),
        )
        .expect("seed legacy session-state.json");
    }

    /// One dormant-on-disk canvas per project: an agent pane the user left
    /// running plus a Shell pane, both persisted as `running`.
    fn seed_workspaces(&self) {
        let _home = gwt_core::test_support::ScopedGwtHome::set(self.home.join(".gwt"));
        for root in [&self.workspace_home, &self.other_home] {
            let path = workspace_state_path(root);
            std::fs::create_dir_all(path.parent().expect("workspace parent"))
                .expect("create project store");
            std::fs::write(
                &path,
                r#"{
  "viewport": { "x": 0.0, "y": 0.0, "zoom": 1.0 },
  "windows": [
    {
      "id": "claude-1",
      "title": "Claude",
      "preset": "claude",
      "geometry": { "x": 0.0, "y": 0.0, "width": 720.0, "height": 420.0 },
      "z_index": 1,
      "status": "running",
      "agent_id": "agent-1"
    },
    {
      "id": "shell-1",
      "title": "Shell",
      "preset": "shell",
      "geometry": { "x": 40.0, "y": 40.0, "width": 720.0, "height": 420.0 },
      "z_index": 2,
      "status": "running"
    }
  ],
  "next_z_index": 3
}"#,
            )
            .expect("seed workspace.json");
        }
    }

    /// The restore `AppRuntime::new` performs, in its production order.
    fn restore(&self) -> Vec<PersistedSessionTabState> {
        let _home = gwt_core::test_support::ScopedGwtHome::set(self.home.join(".gwt"));
        let persisted = load_session_state(&self.session_path()).expect("load legacy session");
        collapse_duplicate_session_tabs(
            persisted.tabs,
            persisted.legacy_active_tab_id.as_deref(),
            gwt_core::paths::project_scope_hash,
        )
    }
}

fn git(args: &[&str], cwd: &Path) {
    let mut command = hidden_command("git");
    command.args(args).current_dir(cwd);
    scrub_git_env(&mut command);
    let output = command.output().expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn init_repo(root: &Path, origin: &str) {
    std::fs::create_dir_all(root).expect("create repository root");
    git(&["init"], root);
    git(&["config", "user.email", "test@example.com"], root);
    git(&["config", "user.name", "Test User"], root);
    git(&["remote", "add", "origin", origin], root);
    std::fs::write(root.join("README.md"), "# fixture\n").expect("seed a commit");
    git(&["add", "README.md"], root);
    git(&["commit", "-m", "init"], root);
}

fn add_worktree(repo: &Path, branch: &str, target: &Path) {
    git(
        &[
            "worktree",
            "add",
            "-b",
            branch,
            target.to_str().expect("worktree path"),
        ],
        repo,
    );
}

/// AC-2: two tabs of one repository must not both own
/// `~/.gwt/projects/<key>/workspace.json`, and the survivor is the one the user
/// was last looking at.
#[test]
fn restoring_a_legacy_session_collapses_duplicate_roots_onto_the_active_tab() {
    let fixture = LegacySessionFixture::new();
    fixture.seed_legacy_session("project-2");

    let restored = fixture.restore();

    let ids: Vec<&str> = restored.iter().map(|tab| tab.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["project-2", "project-3"],
        "the duplicated repository keeps its active tab; the unrelated project is untouched"
    );

    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.join(".gwt"));
    let stores: Vec<PathBuf> = restored
        .iter()
        .map(|tab| workspace_state_path(&tab.project_root))
        .collect();
    assert_ne!(
        stores[0], stores[1],
        "every surviving tab must own a distinct project store"
    );
}

/// AC-5: restoring every unique project must not spawn one agent process per
/// project. The canvas comes back; the agent pane comes back dormant.
#[test]
fn restoring_a_project_workspace_revives_the_canvas_but_not_its_agent_process() {
    let fixture = LegacySessionFixture::new();
    fixture.seed_legacy_session("project-2");
    fixture.seed_workspaces();

    let restored = fixture.restore();
    assert_eq!(restored.len(), 2, "both unique projects must be restored");

    let _home = gwt_core::test_support::ScopedGwtHome::set(fixture.home.join(".gwt"));
    for tab in &restored {
        let workspace =
            load_restored_workspace_state(&tab.project_root).expect("restore project workspace");
        assert_eq!(
            workspace.windows.len(),
            2,
            "the persisted canvas of {} must survive the restore",
            tab.project_root.display()
        );
        for window in &workspace.windows {
            assert_eq!(
                window.status,
                WindowState::Stopped,
                "{} must be restored without a live process behind it",
                window.id
            );
        }
        assert!(
            workspace
                .windows
                .iter()
                .any(|window| window.preset == WindowPreset::Claude),
            "the agent pane itself must still be on the canvas so the manual \
             resume route has something to resume"
        );
    }
}

/// AC-4: the legacy `active_tab_id` is read once (it is the only record of
/// which duplicate the user was on) and never written back.
#[test]
fn the_first_save_after_a_legacy_restore_drops_active_tab_id() {
    let fixture = LegacySessionFixture::new();
    fixture.seed_legacy_session("project-2");

    let loaded = load_session_state(&fixture.session_path()).expect("legacy load");
    assert_eq!(loaded.legacy_active_tab_id.as_deref(), Some("project-2"));

    let mut saved = loaded.clone();
    saved.tabs = fixture.restore();
    saved.legacy_active_tab_id = None;
    save_session_state(&fixture.session_path(), &saved).expect("save restored session");

    let raw = std::fs::read_to_string(fixture.session_path()).expect("read saved session");
    assert!(
        !raw.contains("active_tab_id"),
        "the saved session must not carry active_tab_id: {raw}"
    );
    let reloaded = load_session_state(&fixture.session_path()).expect("reload");
    assert_eq!(reloaded.legacy_active_tab_id, None);
    assert_eq!(
        reloaded
            .tabs
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>(),
        vec!["project-2", "project-3"],
        "the collapse must survive the round trip"
    );
    assert_eq!(reloaded.tabs[0].kind, ProjectKind::Git);
}

/// AC-6: neither entry point may open a browser by itself.
///
/// The OS autostart registration runs the executable with **no** arguments
/// (`AutostartManager` registers the bare path), which is exactly the argv a
/// manual launch uses, so one launch covers both entries. The launch below
/// deliberately omits `--no-open`: with auto-open unsuppressed, a browser that
/// stays closed is the behaviour under test rather than a flag's side effect.
/// `--no-tray` is only the CI display accommodation `stable_server_port` uses.
#[test]
#[ignore = "spawns the real gwt binary; needs a display (CI runs it under xvfb)"]
fn front_door_startup_opens_no_browser() {
    assert!(
        !gwt::cli::tray::parse_tray_argv(&["gwt".to_string()])
            .expect("a bare launch must parse")
            .no_open,
        "the argv both entry points use must leave auto-open unsuppressed, \
         otherwise this test would pass for the wrong reason"
    );

    let fixture = LegacySessionFixture::new();
    fixture.seed_legacy_session("project-2");
    fixture.seed_workspaces();

    let temp_root = fixture._temp.path();
    let shim_dir = temp_root.join("shims");
    let opened_marker = temp_root.join("browser-opened.log");
    std::fs::create_dir_all(&shim_dir).expect("create shim dir");
    for launcher in ["open", "xdg-open"] {
        install_recording_shim(&shim_dir, launcher, &opened_marker);
    }

    let url_path = temp_root.join("browser-url.txt");
    let stdout_path = temp_root.join("stdout.log");
    let stderr_path = temp_root.join("stderr.log");
    let stdout = File::create(&stdout_path).expect("create stdout capture");
    let stderr = File::create(&stderr_path).expect("create stderr capture");

    let path = match std::env::var_os("PATH") {
        Some(existing) => {
            let mut entries = vec![shim_dir.clone()];
            entries.extend(std::env::split_paths(&existing));
            std::env::join_paths(entries).expect("prepend shim dir to PATH")
        }
        None => shim_dir.clone().into_os_string(),
    };

    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwt"));
    scrub_git_env(&mut command);
    command
        .arg("--no-tray")
        .current_dir(&fixture.workspace_home)
        .env("HOME", &fixture.home)
        .env("USERPROFILE", &fixture.home)
        .env("PATH", &path)
        .env("XDG_CONFIG_HOME", fixture.home.join("xdg-config"))
        .env("XDG_CACHE_HOME", fixture.home.join("xdg-cache"))
        .env("XDG_DATA_HOME", fixture.home.join("xdg-data"))
        .env("XDG_STATE_HOME", fixture.home.join("xdg-state"))
        .env("CI", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GWT_BROWSER_URL_FILE", &url_path)
        .env_remove("GWT_SESSION_ID")
        .env_remove("GWT_PROJECT_ROOT")
        .env_remove("GWT_PROJECT_ROOT_HASH")
        .env_remove("GWT_WORKTREE_HASH")
        .env_remove("GWT_WORKSPACE_ID")
        .env_remove("GWT_RUNTIME_DIR")
        .env_remove("GWT_FORCE_NEW_INSTANCE")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    let mut running = RunningGwt(Some(command.spawn().expect("spawn isolated gwt")));
    let url = wait_until_serving(&mut running, &url_path, &stdout_path, &stderr_path);

    assert!(
        !opened_marker.exists(),
        "startup must not open a browser, but a launcher shim was invoked: {}\nurl: {url}\nstderr:\n{}",
        std::fs::read_to_string(&opened_marker).unwrap_or_default(),
        read_capture(&stderr_path),
    );
    drop(running);
    assert!(
        !opened_marker.exists(),
        "shutdown must not open a browser either: {}",
        std::fs::read_to_string(&opened_marker).unwrap_or_default(),
    );

    // Control: the probe above only means something if it can see a browser
    // open at all. Drive the exact production opener through the same shimmed
    // PATH and require the marker this time.
    let _env = gwt_core::test_support::env_lock().lock();
    let _shimmed_path = gwt_core::test_support::ScopedEnvVar::set("PATH", &path);
    gwt::cli::tray::open_browser_for_url(&url).expect("control browser open");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !opened_marker.exists() && Instant::now() < deadline {
        // test-hygiene: allow-short-duration polling interval inside a deadline loop; the bound is the deadline, not this sleep
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        opened_marker.exists(),
        "the launcher shim must record a real browser open, otherwise the \
         assertion above proves nothing",
    );
}

#[cfg(unix)]
fn install_recording_shim(shim_dir: &Path, name: &str, marker: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let path = shim_dir.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '{name} %s\\n' \"$*\" >> {marker}\n",
            marker = shell_quote(marker),
        ),
    )
    .expect("write launcher shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make launcher shim executable");
}

#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

#[cfg(not(unix))]
fn install_recording_shim(shim_dir: &Path, name: &str, marker: &Path) {
    std::fs::write(
        shim_dir.join(format!("{name}.cmd")),
        format!("@echo {name} %* >> \"{}\"\r\n", marker.display()),
    )
    .expect("write launcher shim");
}

struct RunningGwt(Option<Child>);

impl Drop for RunningGwt {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        #[cfg(unix)]
        // SAFETY: the PID comes from this live Child and SIGTERM is the
        // application's supported graceful-shutdown path.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = child.kill();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            // test-hygiene: allow-short-duration polling interval inside a deadline loop; the bound is the deadline, not this sleep
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn wait_until_serving(
    running: &mut RunningGwt,
    url_path: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
) -> String {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .expect("health client");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let child = running.0.as_mut().expect("running child");
        if let Some(status) = child.try_wait().expect("inspect child") {
            panic!(
                "gwt exited before readiness ({status})\nstdout:\n{}\nstderr:\n{}",
                read_capture(stdout_path),
                read_capture(stderr_path),
            );
        }
        if let Ok(raw_url) = std::fs::read_to_string(url_path) {
            let url = raw_url.trim();
            if !url.is_empty()
                && client
                    .get(format!("{url}healthz"))
                    .send()
                    .is_ok_and(|response| response.status().is_success())
            {
                return url.to_string();
            }
        }
        assert!(
            Instant::now() < deadline,
            "gwt did not become ready within {STARTUP_TIMEOUT:?}\nstdout:\n{}\nstderr:\n{}",
            read_capture(stdout_path),
            read_capture(stderr_path),
        );
        // test-hygiene: allow-short-duration polling interval inside a deadline loop; the bound is the deadline, not this sleep
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn read_capture(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| format!("<unreadable: {error}>"))
}
