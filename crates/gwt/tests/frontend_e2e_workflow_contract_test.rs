//! Contract tests for the `Test (Frontend + WebView E2E)` browser bootstrap
//! (Issue #3659).
//!
//! The job used to materialize browsers with a single
//! `npx --yes playwright@<version> install --with-deps chromium` step. That one
//! command hides two very different failure domains behind one log region: an
//! `apt-get` transaction run as root, and a browser download. On
//! 2026-08-18 the `apt-get` half stalled after fetching
//! `noble-security InRelease` and produced no output for 9m31s, until the
//! step timeout killed it:
//!
//! ```text
//! 01:31:23  Get:5 https://archive.ubuntu.com/ubuntu noble-security InRelease [126 kB]
//! 01:40:54  ##[error]The action 'Install Playwright chromium + system deps' has timed out after 10 minutes.
//! 01:40:54  Terminate orphan process: pid (2871) (npm exec playwright@1.49.1 install --with-deps chromium)
//! ```
//!
//! Nothing in the pipeline bounded that `apt-get`, nothing retried it, and the
//! browsers were re-downloaded on every run, so an unrelated PR turned red and
//! could only be recovered by re-running the job. These tests pin the
//! properties that keep an install stall from looking like a test failure.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn test_workflow() -> String {
    read(".github/workflows/test.yml")
}

/// Returns the body of the `test-frontend` job, so assertions cannot be
/// satisfied by an unrelated job elsewhere in the same workflow file.
fn frontend_job() -> String {
    let workflow = test_workflow();
    workflow
        .split("\n  test-frontend:")
        .nth(1)
        .expect("test.yml must keep the test-frontend job")
        .split("\n  test-")
        .next()
        .expect("split always yields a first segment")
        .to_string()
}

/// Returns the body of a single named step inside the `test-frontend` job.
fn frontend_step(name: &str) -> String {
    let job = frontend_job();
    let marker = format!("- name: {name}");
    job.split(&marker)
        .nth(1)
        .unwrap_or_else(|| panic!("test-frontend job must keep the `{name}` step:\n{job}"))
        .split("\n      - ")
        .next()
        .expect("split always yields a first segment")
        .to_string()
}

const INSTALLER: &str = "scripts/install-playwright-browsers.sh";
const VERSION_FILE: &str = "scripts/playwright-version.txt";

// ---------------------------------------------------------------------------
// Workflow contracts
// ---------------------------------------------------------------------------

/// A bare `playwright install --with-deps` has no timeout of its own, so a
/// stalled apt mirror can only be stopped by the step timeout — 10 minutes of
/// silence that reads as "CI is broken" rather than "the install hung".
#[test]
fn playwright_bootstrap_delegates_to_the_bounded_installer_script() {
    let job = frontend_job();
    // Only executed commands matter here; the surrounding comments are allowed
    // to name the command this replaced.
    let commands: Vec<&str> = job
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect();

    assert!(
        !commands
            .iter()
            .any(|line| line.contains("install --with-deps")),
        "the frontend job must not run a bare `playwright install --with-deps`; \
         it merges the apt transaction and the browser download into one \
         unbounded, unretried command:\n{job}"
    );
    assert!(
        job.contains(INSTALLER),
        "the frontend job must bootstrap browsers through {INSTALLER}, which \
         bounds and retries each phase:\n{job}"
    );
}

/// The apt transaction and the browser download fail for unrelated reasons
/// (mirror stall vs. CDN error). Keeping them in separate steps means the
/// GitHub error line already names the phase that broke.
#[test]
fn system_deps_and_browser_download_are_separate_steps() {
    let job = frontend_job();

    let deps_step = frontend_step("Install Playwright system dependencies");
    let browser_step = frontend_step("Install Playwright chromium browser");

    assert!(
        deps_step.contains("system-deps"),
        "the system dependency step must run the installer's `system-deps` \
         phase:\n{deps_step}"
    );
    assert!(
        browser_step.contains("browsers"),
        "the browser step must run the installer's `browsers` phase:\n{browser_step}"
    );

    let deps_at = job
        .find("- name: Install Playwright system dependencies")
        .expect("system dependency step must exist");
    let browser_at = job
        .find("- name: Install Playwright chromium browser")
        .expect("browser step must exist");
    assert!(
        deps_at < browser_at,
        "system dependencies must be installed before the browser download"
    );
}

/// Both phases talk to the network. Without a per-step budget one stalled
/// phase consumes the whole job's time before anything is reported.
#[test]
fn each_bootstrap_step_carries_its_own_timeout() {
    for step in [
        "Install Playwright system dependencies",
        "Install Playwright chromium browser",
    ] {
        let body = frontend_step(step);
        assert!(
            body.contains("timeout-minutes:"),
            "`{step}` must declare its own timeout-minutes so a stall is \
             attributed to that phase:\n{body}"
        );
    }
}

/// Re-downloading chromium on every run makes every job depend on the
/// Playwright CDN. A cache turns the common path into a no-op, so the
/// download can only fail when the cache genuinely misses.
#[test]
fn playwright_browsers_are_cached_across_runs() {
    let job = frontend_job();

    assert!(
        job.contains("actions/cache@"),
        "the frontend job must cache the Playwright browser bundle:\n{job}"
    );
    assert!(
        job.contains("~/.cache/ms-playwright"),
        "the cache must cover ~/.cache/ms-playwright, where Playwright stores \
         downloaded browsers on Linux:\n{job}"
    );
}

/// A cache key built from a version literal that has drifted from the pinned
/// version restores browsers the tests cannot use, and the download runs
/// anyway — reintroducing the failure the cache was added to remove.
#[test]
fn the_workflow_does_not_duplicate_the_pinned_playwright_version() {
    let workflow = test_workflow();

    assert!(
        !workflow.contains("playwright@1."),
        "the workflow must not pin a Playwright version literal; the pinned \
         version lives in {VERSION_FILE} so the cache key cannot drift away \
         from what the tests actually run:\n{workflow}"
    );
    assert!(
        workflow.contains(VERSION_FILE),
        "the workflow must read the pinned version from {VERSION_FILE}:\n{workflow}"
    );
}

/// AC-4: an install stall and a failing spec must never share a log region.
#[test]
fn the_e2e_run_stays_a_separate_step_from_the_bootstrap() {
    let job = frontend_job();
    let e2e_at = job
        .find("- name: Run WebView E2E")
        .expect("the WebView E2E step must exist");
    let browser_at = job
        .find("- name: Install Playwright chromium browser")
        .expect("browser step must exist");

    assert!(
        browser_at < e2e_at,
        "browsers must be installed in their own step before the E2E run, so \
         a bootstrap failure never appears inside the test output"
    );
    assert!(
        frontend_step("Run WebView E2E (embedded Playwright specs)")
            .contains("run-visual-tests.sh"),
        "the E2E step must only run the specs"
    );
}

// ---------------------------------------------------------------------------
// Pinned-version single source
// ---------------------------------------------------------------------------

/// `run-visual-tests.sh` installs `@playwright/test` at the pinned version and
/// resolves browsers from the shared cache. If it pins a different version
/// than the bootstrap, the cached browser revision does not match and every
/// run downloads again.
#[test]
fn the_pinned_version_has_exactly_one_source() {
    let version = read(VERSION_FILE);
    let version = version.trim();
    assert!(
        !version.is_empty(),
        "{VERSION_FILE} must contain the pinned Playwright version"
    );

    for script in ["scripts/run-visual-tests.sh", INSTALLER] {
        let body = read(script);
        assert!(
            body.contains("playwright-version.txt"),
            "{script} must read the pinned version from {VERSION_FILE} instead \
             of carrying its own literal:\n{body}"
        );
    }
}

// ---------------------------------------------------------------------------
// Installer behaviour
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod installer {
    use super::*;
    use gwt_core::process::{resolved_command, ProcessPlanRequest};
    use std::os::unix::fs::PermissionsExt;
    use std::process::Output;

    struct Harness {
        dir: PathBuf,
    }

    impl Harness {
        /// Builds a fake Playwright CLI whose body is `script`, plus the
        /// scratch directories the installer writes into.
        fn new(name: &str, script: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "gwt-3659-{name}-{}-{}",
                std::process::id(),
                name.len()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create harness dir");
            fs::create_dir_all(dir.join("apt.conf.d")).expect("create apt conf dir");

            let cli = dir.join("fake-playwright");
            fs::write(&cli, script).expect("write fake playwright CLI");
            let mut perms = fs::metadata(&cli).expect("stat fake CLI").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&cli, perms).expect("chmod fake CLI");

            Self { dir }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.dir.join(relative)
        }

        fn run(&self, phase: &str, extra_env: &[(&str, &str)]) -> Output {
            let installer = repo_root().join(INSTALLER);
            let mut command = resolved_command(
                ProcessPlanRequest::new("bash").args([installer.to_string_lossy().as_ref(), phase]),
            )
            .expect("resolve bash");
            command
                .env("GWT_PLAYWRIGHT_CLI", self.path("fake-playwright"))
                .env("GWT_PLAYWRIGHT_APT_CONF_DIR", self.path("apt.conf.d"))
                .env("GWT_PLAYWRIGHT_SYSTEM_DEPS", "always")
                .env("GWT_PLAYWRIGHT_RETRY_DELAY", "0")
                .env("GWT_PLAYWRIGHT_INSTALL_RETRIES", "3")
                .env("GWT_PLAYWRIGHT_INSTALL_TIMEOUT", "5")
                .env("GWT_PLAYWRIGHT_STATE_DIR", self.path("state"));
            for (key, value) in extra_env {
                command.env(key, value);
            }
            command.output().expect("run installer")
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn combined(output: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    const ALWAYS_OK: &str = "#!/usr/bin/env bash\nexit 0\n";

    /// AC-4 at the log level: even inside one step, the two phases must be
    /// individually greppable, because the workflow may run them together
    /// locally.
    #[test]
    fn each_phase_announces_itself_before_running() {
        let harness = Harness::new("phases", ALWAYS_OK);
        let output = harness.run("all", &[]);
        let log = combined(&output);

        assert!(output.status.success(), "installer must succeed:\n{log}");
        assert!(
            log.contains("phase=system-deps"),
            "the apt phase must be labelled in the log:\n{log}"
        );
        assert!(
            log.contains("phase=browsers"),
            "the browser download phase must be labelled in the log:\n{log}"
        );
    }

    /// AC-2: a transient CDN or mirror error must not fail the job, and the
    /// retry must be visible so a flaky bootstrap is distinguishable from a
    /// healthy one.
    #[test]
    fn a_transient_failure_is_retried_and_every_attempt_is_logged() {
        let harness = Harness::new("retry", FAIL_TWICE);
        let output = harness.run("browsers", &[]);
        let log = combined(&output);

        assert!(
            output.status.success(),
            "the installer must recover from transient failures:\n{log}"
        );
        assert!(
            log.contains("attempt=1/3") && log.contains("attempt=3/3"),
            "every attempt must be logged with its ordinal:\n{log}"
        );
    }

    /// A retried-forever bootstrap is the original bug with extra steps. Once
    /// the budget is spent the installer must fail and name the phase.
    #[test]
    fn exhausting_the_retries_fails_and_names_the_phase() {
        let harness = Harness::new("exhausted", "#!/usr/bin/env bash\nexit 1\n");
        let output = harness.run("browsers", &[]);
        let log = combined(&output);

        assert!(
            !output.status.success(),
            "the installer must fail once the retry budget is spent:\n{log}"
        );
        assert!(
            log.contains("phase=browsers") && log.contains("3 attempts"),
            "the failure must name the phase and the exhausted budget:\n{log}"
        );
    }

    /// AC-1, and the direct fix for the observed hang: an attempt that never
    /// returns must be killed by the installer, well before the step timeout
    /// turns it into an opaque `The action ... has timed out`.
    #[test]
    fn a_hanging_attempt_is_killed_by_the_installers_own_timeout() {
        // `exec` so the killed process *is* the sleep: a grandchild would
        // outlive the kill and keep the captured pipes open.
        let harness = Harness::new("hang", "#!/usr/bin/env bash\nexec sleep 600\n");
        let started = std::time::Instant::now();
        let output = harness.run(
            "browsers",
            &[
                ("GWT_PLAYWRIGHT_INSTALL_TIMEOUT", "2"),
                ("GWT_PLAYWRIGHT_INSTALL_RETRIES", "2"),
            ],
        );
        let elapsed = started.elapsed();
        let log = combined(&output);

        assert!(
            !output.status.success(),
            "a hanging install must fail, not hang:\n{log}"
        );
        assert!(
            elapsed.as_secs() < 60,
            "the installer must stop a hanging attempt on its own budget, \
             took {elapsed:?}:\n{log}"
        );
        assert!(
            log.contains("timed out"),
            "the log must say the attempt timed out, so the phase is not \
             mistaken for a test failure:\n{log}"
        );
    }

    /// Root cause: `apt-get` had no acquire timeout, so a stalled mirror
    /// blocked forever. Bounding acquire turns the stall into a fast, retryable
    /// error.
    #[test]
    fn apt_is_bounded_so_a_stalled_mirror_fails_fast() {
        let harness = Harness::new("apt", ALWAYS_OK);
        let output = harness.run("system-deps", &[]);
        let log = combined(&output);
        assert!(output.status.success(), "installer must succeed:\n{log}");

        let conf_dir = harness.path("apt.conf.d");
        let conf = read_only_entry(&conf_dir);

        for setting in [
            "Acquire::http::Timeout",
            "Acquire::https::Timeout",
            "Acquire::Retries",
            "DPkg::Lock::Timeout",
        ] {
            assert!(
                conf.contains(setting),
                "apt must be configured with {setting} so a stalled mirror \
                 or a held dpkg lock cannot hang the job:\n{conf}"
            );
        }
    }

    /// Issue #3701: Ubuntu runners often hold the dpkg lock for 1–3 minutes
    /// while unattended-upgrades finishes. A 10s retry gap is shorter than
    /// that window, so three immediate `Could not get lock` failures look
    /// like a hard bootstrap break.
    #[test]
    fn system_deps_retry_delay_covers_a_background_apt_window() {
        let delay = default_numeric_env(INSTALLER, "GWT_PLAYWRIGHT_RETRY_DELAY")
            .expect("installer must default GWT_PLAYWRIGHT_RETRY_DELAY");
        assert!(
            delay >= 30,
            "system-deps retry delay must cover a typical unattended-upgrades \
             window (at least 30s), found {delay}s in {INSTALLER}"
        );
    }

    /// Issue #3701: a lock-held apt-get fails in a few milliseconds. The
    /// log used to say only `status=failed exit=1`, so the next person
    /// bisecting a red PR could not tell the runner was busy.
    #[test]
    fn a_dpkg_lock_error_is_named_in_the_log() {
        let harness = Harness::new("dpkg-lock", DPKG_LOCK_FAIL);
        let output = harness.run("system-deps", &[("GWT_PLAYWRIGHT_INSTALL_RETRIES", "2")]);
        let log = combined(&output);

        assert!(
            !output.status.success(),
            "a held dpkg lock that never clears must fail the phase:\n{log}"
        );
        assert!(
            log.contains("reason=dpkg lock contention"),
            "the log must name the lock contention so a red PR is not \
             mistaken for a product regression:\n{log}"
        );
    }

    /// Issue #3701: wait for the lock *before* spending an install attempt,
    /// so a typical unattended-upgrades window is absorbed instead of
    /// burning the retry budget on instant failures.
    #[test]
    fn system_deps_waits_for_a_held_dpkg_lock_before_apt() {
        let harness = Harness::new("lock-wait", ALWAYS_OK);
        let probe = harness.path("lock-probe");
        write_executable(
            &probe,
            r#"#!/usr/bin/env bash
counter="${GWT_PLAYWRIGHT_STATE_DIR}/lock-probes"
mkdir -p "${GWT_PLAYWRIGHT_STATE_DIR}"
printf 'x' >> "${counter}"
n=$(wc -c < "${counter}" | tr -d ' ')
# First two probes: lock held. Afterwards: released.
if [ "${n}" -le 2 ]; then
  exit 0
fi
exit 1
"#,
        );
        let output = harness.run(
            "system-deps",
            &[
                ("GWT_APT_LOCK_PROBE", probe.to_string_lossy().as_ref()),
                ("GWT_APT_LOCK_POLL", "1"),
                ("GWT_APT_LOCK_TIMEOUT", "10"),
            ],
        );
        let log = combined(&output);

        assert!(
            output.status.success(),
            "system-deps must wait out a short-lived dpkg lock:\n{log}"
        );
        assert!(
            log.contains("waiting for dpkg lock"),
            "waiting on the lock must be visible in the log:\n{log}"
        );
        assert!(
            log.contains("phase=system-deps attempt=1"),
            "the apt phase must still run after the lock is released:\n{log}"
        );
    }

    /// A lock that never clears must fail on the wait budget, not hang
    /// until the GitHub step timeout.
    #[test]
    fn a_held_dpkg_lock_fails_within_the_wait_budget() {
        let harness = Harness::new("lock-timeout", ALWAYS_OK);
        let probe = harness.path("lock-probe");
        write_executable(&probe, "#!/usr/bin/env bash\nexit 0\n");
        let started = std::time::Instant::now();
        let output = harness.run(
            "system-deps",
            &[
                ("GWT_APT_LOCK_PROBE", probe.to_string_lossy().as_ref()),
                ("GWT_APT_LOCK_POLL", "1"),
                ("GWT_APT_LOCK_TIMEOUT", "2"),
            ],
        );
        let elapsed = started.elapsed();
        let log = combined(&output);

        assert!(
            !output.status.success(),
            "a lock that never clears must fail:\n{log}"
        );
        assert!(
            elapsed.as_secs() < 30,
            "the wait must stop on its own budget, took {elapsed:?}:\n{log}"
        );
        assert!(
            log.contains("reason=dpkg lock contention"),
            "the failure must name the lock contention:\n{log}"
        );
    }

    /// `--with-deps` shells out to `apt-get install`. Any interactive prompt
    /// (apt itself, or Ubuntu 24.04's needrestart) waits for input that never
    /// arrives on a runner.
    #[test]
    fn apt_never_waits_for_an_interactive_answer() {
        let harness = Harness::new("noninteractive", ENV_DUMP);
        let output = harness.run("system-deps", &[]);
        let log = combined(&output);
        assert!(output.status.success(), "installer must succeed:\n{log}");

        let env_dump = fs::read_to_string(harness.path("state/env")).expect("fake CLI env dump");
        assert!(
            env_dump.contains("DEBIAN_FRONTEND=noninteractive"),
            "apt must run non-interactively:\n{env_dump}"
        );
        assert!(
            env_dump.contains("NEEDRESTART_MODE=a"),
            "needrestart must not prompt on Ubuntu 24.04:\n{env_dump}"
        );
    }

    /// The apt drop-in is a fast-fail optimization, not a prerequisite: the
    /// per-attempt timeout already prevents the hang. Refusing to bootstrap
    /// because the drop-in could not be written would turn a slow path into no
    /// path at all.
    #[test]
    fn an_unwritable_apt_config_does_not_block_the_bootstrap() {
        let harness = Harness::new("apt-missing", ALWAYS_OK);
        let missing = harness.path("no-such-apt-conf-dir");
        let output = harness.run(
            "system-deps",
            &[(
                "GWT_PLAYWRIGHT_APT_CONF_DIR",
                missing.to_string_lossy().as_ref(),
            )],
        );
        let log = combined(&output);

        assert!(
            output.status.success(),
            "the bootstrap must continue when apt cannot be hardened:\n{log}"
        );
        assert!(
            log.contains("apt-hardening=skipped"),
            "skipping the apt drop-in must be recorded, so a later hang is \
             attributable:\n{log}"
        );
        assert!(
            log.contains("phase=system-deps attempt=1/3"),
            "the apt phase must still run, still bounded:\n{log}"
        );
    }

    /// The installer is also the local-development entrypoint, so it must not
    /// attempt an apt transaction on a machine that has no apt.
    #[test]
    fn the_apt_phase_is_skipped_where_apt_does_not_exist() {
        let harness = Harness::new("skip", ALWAYS_OK);
        let output = harness.run("all", &[("GWT_PLAYWRIGHT_SYSTEM_DEPS", "never")]);
        let log = combined(&output);

        assert!(output.status.success(), "installer must succeed:\n{log}");
        assert!(
            log.contains("phase=system-deps") && log.contains("skipped"),
            "skipping the apt phase must still be recorded in the log:\n{log}"
        );
        assert!(
            log.contains("phase=browsers"),
            "the browser phase must still run:\n{log}"
        );
    }

    fn read_only_entry(dir: &Path) -> String {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .expect("read apt conf dir")
            .map(|entry| entry.expect("dir entry").path())
            .collect();
        entries.sort();
        assert_eq!(
            entries.len(),
            1,
            "the installer must drop exactly one apt config file, found {entries:?}"
        );
        fs::read_to_string(&entries[0]).expect("read apt config")
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap_or_else(|error| {
            panic!("write {}: {error}", path.display());
        });
        let mut perms = fs::metadata(path)
            .unwrap_or_else(|error| panic!("stat {}: {error}", path.display()))
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap_or_else(|error| {
            panic!("chmod {}: {error}", path.display());
        });
    }

    fn default_numeric_env(relative: &str, name: &str) -> Option<u64> {
        let marker = format!("{name}:-");
        let body = read(relative);
        for line in body.lines() {
            let Some(start) = line.find(&marker) else {
                continue;
            };
            let rest = &line[start + marker.len()..];
            let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            if let Ok(value) = digits.parse() {
                return Some(value);
            }
        }
        None
    }

    const FAIL_TWICE: &str = r#"#!/usr/bin/env bash
counter="${GWT_PLAYWRIGHT_STATE_DIR}/attempts"
mkdir -p "${GWT_PLAYWRIGHT_STATE_DIR}"
printf 'x' >> "${counter}"
attempts=$(wc -c < "${counter}" | tr -d ' ')
if [ "${attempts}" -lt 3 ]; then
  echo "transient failure ${attempts}" >&2
  exit 1
fi
exit 0
"#;

    const ENV_DUMP: &str = r#"#!/usr/bin/env bash
mkdir -p "${GWT_PLAYWRIGHT_STATE_DIR}"
env > "${GWT_PLAYWRIGHT_STATE_DIR}/env"
exit 0
"#;

    const DPKG_LOCK_FAIL: &str = r#"#!/usr/bin/env bash
echo "E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 3442 (apt-get)" >&2
echo "E: Unable to acquire the dpkg frontend lock (/var/lib/dpkg/lock-frontend), is another process using it?" >&2
exit 1
"#;
}
