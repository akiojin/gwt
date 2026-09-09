//! Contract tests for CI apt lock waiting (Issue #3701).
//!
//! GitHub-hosted Ubuntu images often run `unattended-upgrades` just after
//! boot and hold `/var/lib/dpkg/lock-frontend`. Two failure modes showed up
//! on 2026-08-19:
//!
//! 1. Playwright `install-deps` failed immediately with `Could not get lock`
//!    and exhausted 10s × 3 retries while the background apt was still
//!    running.
//! 2. The GTK `apt-get` step in `Test (Rust)` had no `timeout-minutes` and
//!    waited on the lock until the job was cancelled by hand.
//!
//! These tests pin the shared wrapper and the workflow wiring that keep a
//! held dpkg lock from hanging a job or failing a PR for an unrelated change.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Output;
use std::time::Instant;

use gwt_core::process::{resolved_command, ProcessPlanRequest};

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

const CI_APT: &str = "scripts/ci-apt.sh";
const LINUX_DEPS: &str = "scripts/install-linux-deps.sh";

fn workflow_paths() -> Vec<PathBuf> {
    let dir = repo_root().join(".github/workflows");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.expect("workflow dir entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .collect();
    paths.sort();
    paths
}

/// A workflow step whose `run:` body talks to apt, including the shared
/// wrapper and the Playwright installer that shells out to apt-get.
struct AptStep {
    workflow: PathBuf,
    name: String,
    body: String,
}

fn named_steps(workflow: &str) -> Vec<(String, String)> {
    let mut steps = Vec::new();
    let mut rest = workflow;
    while let Some(at) = rest.find("\n      - name:") {
        rest = &rest[at + 1..];
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let name = rest["      - name:".len()..line_end].trim().to_string();
        let after = &rest[line_end..];
        let body = after
            .split("\n      - ")
            .next()
            .expect("split always yields a first segment")
            .to_string();
        steps.push((name, body));
    }
    steps
}

fn apt_steps() -> Vec<AptStep> {
    let mut steps = Vec::new();
    for path in workflow_paths() {
        let relative = path
            .strip_prefix(repo_root())
            .unwrap_or(&path)
            .to_path_buf();
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for (name, body) in named_steps(&content) {
            let mentions_apt = body.lines().any(|line| {
                let trimmed = line.trim();
                !trimmed.starts_with('#')
                    && (trimmed.contains("apt-get")
                        || trimmed.contains("ci-apt.sh")
                        || trimmed.contains("install-linux-deps.sh")
                        || trimmed.contains("install-playwright-browsers.sh"))
            });
            if mentions_apt {
                steps.push(AptStep {
                    workflow: relative.clone(),
                    name,
                    body,
                });
            }
        }
    }
    steps
}

fn executable_lines(body: &str) -> Vec<&str> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// AC: every GitHub step that talks to apt waits on the lock through the
/// shared wrapper (or the Playwright installer) and has its own
/// `timeout-minutes`, so a held dpkg lock cannot hang the job.
#[test]
fn every_github_apt_step_is_lock_aware_and_time_bounded() {
    let steps = apt_steps();
    assert!(
        !steps.is_empty(),
        "the workflows must still install Linux packages via apt"
    );

    let mut failures = Vec::new();
    for step in &steps {
        let lines = executable_lines(&step.body);
        let uses_raw_apt = lines.iter().any(|line| line.contains("apt-get"));
        let uses_wrapper = lines.iter().any(|line| {
            line.contains("ci-apt.sh") || line.contains("install-playwright-browsers.sh")
        });
        if uses_raw_apt {
            failures.push(format!(
                "{} / `{}` still calls apt-get directly; route it through {CI_APT} \
                 so DPkg::Lock::Timeout and lock waiting stay in one place:\n{}",
                step.workflow.display(),
                step.name,
                step.body
            ));
        }
        if !uses_wrapper {
            failures.push(format!(
                "{} / `{}` talks to apt without {CI_APT} or the Playwright \
                 installer:\n{}",
                step.workflow.display(),
                step.name,
                step.body
            ));
        }
        if !step.body.contains("timeout-minutes:") {
            failures.push(format!(
                "{} / `{}` has no timeout-minutes, so a held dpkg lock can \
                 occupy the runner until someone cancels the job:\n{}",
                step.workflow.display(),
                step.name,
                step.body
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "apt-using GitHub steps must wait on the dpkg lock and bound the wait:\n{}",
        failures.join("\n\n")
    );
}

/// The Docker / local Linux helper is the other apt entrypoint. If it keeps
/// a bare `apt-get`, image builds on a busy host hit the same instant-fail
/// or unbounded-wait path.
#[test]
fn install_linux_deps_uses_the_lock_aware_wrapper() {
    let body = read(LINUX_DEPS);
    assert!(
        body.contains("ci-apt.sh"),
        "{LINUX_DEPS} must install packages through {CI_APT}:\n{body}"
    );
    let raw = body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.starts_with('#') && trimmed.contains("apt-get")
    });
    assert!(!raw, "{LINUX_DEPS} must not call apt-get directly:\n{body}");
}

/// The wrapper itself is the one place that passes DPkg::Lock::Timeout.
#[test]
fn ci_apt_wrapper_exists_and_waits_on_the_dpkg_lock() {
    let path = repo_root().join(CI_APT);
    assert!(
        path.is_file(),
        "{CI_APT} must exist as the shared apt lock wrapper"
    );
    let body = read(CI_APT);
    assert!(
        body.contains("DPkg::Lock::Timeout"),
        "{CI_APT} must pass DPkg::Lock::Timeout so apt waits for a held lock \
         instead of failing immediately:\n{body}"
    );
    assert!(
        body.contains("reason=dpkg lock contention"),
        "{CI_APT} must name lock contention in its log:\n{body}"
    );
}

#[cfg(unix)]
mod wrapper {
    use super::*;

    struct Harness {
        dir: PathBuf,
    }

    impl Harness {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "gwt-3701-{name}-{}-{}",
                std::process::id(),
                name.len()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create harness dir");
            Self { dir }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.dir.join(relative)
        }

        fn write_executable(&self, relative: &str, body: &str) -> PathBuf {
            let path = self.path(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(&path, body).expect("write executable");
            let mut perms = fs::metadata(&path).expect("stat executable").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).expect("chmod executable");
            path
        }

        fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
            let script = repo_root().join(CI_APT);
            let mut argv = vec![script.to_string_lossy().into_owned()];
            argv.extend(args.iter().map(|arg| (*arg).to_string()));
            let mut command =
                resolved_command(ProcessPlanRequest::new("bash").args(argv)).expect("resolve bash");
            command.env("GWT_APT_STATE_DIR", self.path("state"));
            for (key, value) in extra_env {
                command.env(key, value);
            }
            command.output().expect("run ci-apt.sh")
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

    fn fake_apt_get() -> &'static str {
        r#"#!/usr/bin/env bash
mkdir -p "${GWT_APT_STATE_DIR}"
printf '%s\n' "$*" > "${GWT_APT_STATE_DIR}/argv"
exit 0
"#
    }

    /// When the lock is free, the wrapper must still tell apt to wait, so a
    /// lock that appears mid-transaction does not fail the step instantly.
    #[test]
    fn a_free_lock_runs_apt_get_with_lock_timeout() {
        let harness = Harness::new("free");
        let apt_get = harness.write_executable("fake-apt-get", fake_apt_get());
        let probe = harness.write_executable("probe", "#!/usr/bin/env bash\nexit 1\n");
        let output = harness.run(
            &["update"],
            &[
                ("GWT_APT_GET", apt_get.to_string_lossy().as_ref()),
                ("GWT_APT_LOCK_PROBE", probe.to_string_lossy().as_ref()),
                ("GWT_APT_LOCK_TIMEOUT", "12"),
            ],
        );
        let log = combined(&output);
        assert!(output.status.success(), "ci-apt must succeed:\n{log}");

        let argv = fs::read_to_string(harness.path("state/argv")).expect("read argv");
        assert!(
            argv.contains("DPkg::Lock::Timeout=12"),
            "apt-get must be invoked with DPkg::Lock::Timeout:\n{argv}\n{log}"
        );
        assert!(
            argv.contains("update"),
            "the original apt-get arguments must be preserved:\n{argv}"
        );
    }

    /// A lock that clears after a couple of polls must be waited out, then
    /// the real apt-get command runs.
    #[test]
    fn a_short_lived_lock_is_waited_out_then_apt_runs() {
        let harness = Harness::new("wait");
        let apt_get = harness.write_executable("fake-apt-get", fake_apt_get());
        let probe = harness.write_executable(
            "probe",
            r#"#!/usr/bin/env bash
mkdir -p "${GWT_APT_STATE_DIR}"
counter="${GWT_APT_STATE_DIR}/probes"
printf 'x' >> "${counter}"
n=$(wc -c < "${counter}" | tr -d ' ')
if [ "${n}" -le 2 ]; then
  exit 0
fi
exit 1
"#,
        );
        let output = harness.run(
            &["install", "-y", "libgtk-3-dev"],
            &[
                ("GWT_APT_GET", apt_get.to_string_lossy().as_ref()),
                ("GWT_APT_LOCK_PROBE", probe.to_string_lossy().as_ref()),
                ("GWT_APT_LOCK_POLL", "1"),
                ("GWT_APT_LOCK_TIMEOUT", "10"),
            ],
        );
        let log = combined(&output);
        assert!(
            output.status.success(),
            "ci-apt must wait out a short-lived lock:\n{log}"
        );
        assert!(
            log.contains("waiting for dpkg lock"),
            "the wait must be logged:\n{log}"
        );
        let argv = fs::read_to_string(harness.path("state/argv")).expect("read argv");
        assert!(
            argv.contains("install") && argv.contains("libgtk-3-dev"),
            "apt-get must run after the lock is released:\n{argv}"
        );
    }

    /// A lock that never clears must fail with a greppable reason and stop
    /// well before a GitHub step timeout.
    #[test]
    fn a_held_lock_fails_fast_with_contention_reason() {
        let harness = Harness::new("held");
        let apt_get = harness.write_executable("fake-apt-get", fake_apt_get());
        let probe = harness.write_executable("probe", "#!/usr/bin/env bash\nexit 0\n");
        let started = Instant::now();
        let output = harness.run(
            &["update"],
            &[
                ("GWT_APT_GET", apt_get.to_string_lossy().as_ref()),
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
            "the wait must be bounded, took {elapsed:?}:\n{log}"
        );
        assert!(
            log.contains("reason=dpkg lock contention"),
            "the failure must name the lock contention:\n{log}"
        );
        assert!(
            !harness.path("state/argv").exists(),
            "apt-get must not run while the lock is held"
        );
    }
}
