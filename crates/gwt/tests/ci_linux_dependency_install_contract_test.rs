//! Contract tests for the Linux dependency install step (Issue #4191).
//!
//! `Install tray + GTK dependencies (Linux)` is the first step of `Test
//! (Rust)`, the only job that runs `cargo test --workspace --all-features`.
//! When it died on its 5-minute step cap the job reported FAILURE with zero
//! test output, which is indistinguishable from a code failure — PR #4099 was
//! nearly triaged as CI-RED before someone read the log.
//!
//! Across the last 60 green `Test` runs the step measured p50 29s, p90 48s,
//! p95 60s and max 106s, so the old cap was ~3x the worst healthy run: a step
//! that hits it is a stalled mirror, not a slow one, and raising the cap alone
//! would not fix it. The deadline therefore lives inside `scripts/ci-apt.sh`,
//! which retries, reuses a cached .deb archive, and reports a dependency
//! failure as a dependency failure. The workflow `timeout-minutes` is only the
//! outer net, and these tests pin that it stays above the script's own budget
//! — otherwise the step clock kills the script before it can say why.

use std::fs;
use std::path::{Path, PathBuf};

const CI_APT: &str = "scripts/ci-apt.sh";
const DEP_STEP: &str = "Install tray + GTK dependencies (Linux)";
const CACHE_STEP: &str = "Cache Linux GTK dependency packages";
const CACHE_ENV: &str = "GWT_APT_CACHE_DIR";
const GTK_DEPS_MODE: &str = "gtk-deps";

/// Seconds the step cap must clear the script's own deadline by, covering
/// checkout-relative process start, `sudo`, and the `timeout` kill grace.
const MIN_HEADROOM_SECONDS: u64 = 120;

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

fn named_steps(workflow: &str) -> Vec<(String, String)> {
    let mut steps = Vec::new();
    let mut rest = workflow;
    while let Some(at) = rest.find("\n      - name:") {
        rest = &rest[at + 1..];
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let name = rest["      - name:".len()..line_end].trim().to_string();
        let body = rest[line_end..]
            .split("\n      - ")
            .next()
            .expect("split always yields a first segment")
            .to_string();
        steps.push((name, body));
    }
    steps
}

fn timeout_minutes(body: &str) -> Option<u64> {
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("timeout-minutes:"))
        .and_then(|value| value.trim().parse().ok())
}

/// Reads a `${NAME:-default}` fallback out of the shell script, so the YAML
/// budget is derived from the script's real defaults instead of a number
/// copied into the test and left to drift.
fn shell_default(body: &str, name: &str) -> u64 {
    let needle = format!("${{{name}:-");
    let at = body
        .find(&needle)
        .unwrap_or_else(|| panic!("{CI_APT} must define a default for {name}"));
    let rest = &body[at + needle.len()..];
    let end = rest
        .find('}')
        .unwrap_or_else(|| panic!("{CI_APT} default for {name} is unterminated"));
    rest[..end]
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("{CI_APT} default for {name} is not a number: {error}"))
}

/// Every workflow step that installs the Linux GTK dependencies, paired with
/// the full workflow text it came from.
fn dependency_install_steps() -> Vec<(PathBuf, String, String)> {
    let mut found = Vec::new();
    for path in workflow_paths() {
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for (name, body) in named_steps(&content) {
            if name == DEP_STEP {
                let relative = path
                    .strip_prefix(repo_root())
                    .unwrap_or(&path)
                    .to_path_buf();
                found.push((relative, body, content.clone()));
            }
        }
    }
    found
}

fn display(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// AC-1 + AC-3: the step cap is the outer net, never the operative limit. It
/// must stay above the script's own total deadline, because a step killed by
/// GitHub cannot emit the marker that separates a dependency-install failure
/// from a test failure.
#[test]
fn dependency_install_step_cap_stays_above_the_scripts_own_deadline() {
    let script = read(CI_APT);
    let deadline = shell_default(&script, "GWT_APT_TOTAL_DEADLINE");
    let required = deadline + MIN_HEADROOM_SECONDS;

    let steps = dependency_install_steps();
    assert!(
        !steps.is_empty(),
        "the workflows must still install the Linux GTK dependencies"
    );

    let mut failures = Vec::new();
    for (workflow, body, _) in &steps {
        let minutes = timeout_minutes(body).unwrap_or_else(|| {
            panic!(
                "{} / `{DEP_STEP}` must declare timeout-minutes:\n{body}",
                display(workflow)
            )
        });
        let seconds = minutes * 60;
        if seconds < required {
            failures.push(format!(
                "{} / `{DEP_STEP}` allows {seconds}s but {CI_APT} may spend \
                 {deadline}s of its own before giving up; the step needs at \
                 least {required}s ({} minutes) so the script fails first and \
                 can report the failure as a dependency install failure",
                display(workflow),
                required.div_ceil(60)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the GitHub step cap must never preempt the script deadline:\n{}",
        failures.join("\n\n")
    );
}

/// AC-4: the retry budget is explicit and actually fits inside the deadline,
/// so "retries 3 times" is a promise the script can keep rather than a
/// setting the total deadline silently truncates on the first attempt.
#[test]
fn retry_budget_is_explicit_and_fits_inside_the_total_deadline() {
    let script = read(CI_APT);
    let attempts = shell_default(&script, "GWT_APT_ATTEMPTS");
    let attempt_timeout = shell_default(&script, "GWT_APT_ATTEMPT_TIMEOUT");
    let retry_delay = shell_default(&script, "GWT_APT_RETRY_DELAY");
    let deadline = shell_default(&script, "GWT_APT_TOTAL_DEADLINE");

    assert!(
        attempts >= 2,
        "{CI_APT} must retry a failed dependency install at least once \
         (GWT_APT_ATTEMPTS is {attempts}); a network-bound flake otherwise \
         erases the whole workspace test run"
    );

    let spent = attempts * attempt_timeout + attempts.saturating_sub(1) * retry_delay;
    assert!(
        spent <= deadline,
        "{attempts} attempts of {attempt_timeout}s plus {} delays of \
         {retry_delay}s need {spent}s, but GWT_APT_TOTAL_DEADLINE is \
         {deadline}s; the deadline would cut the advertised retry budget short",
        attempts.saturating_sub(1)
    );
}

/// AC-2: the dependency download is cached. Every workflow that installs the
/// GTK dependencies hands the script a cache directory and restores it with
/// `actions/cache`, keyed on the script that owns the package list so a
/// changed package set cannot be served a stale archive.
#[test]
fn dependency_install_is_cached_and_keyed_on_the_package_set() {
    let steps = dependency_install_steps();
    assert!(
        !steps.is_empty(),
        "the workflows must still install the Linux GTK dependencies"
    );

    let mut failures = Vec::new();
    for (workflow, body, workflow_text) in &steps {
        if !body.contains(CACHE_ENV) {
            failures.push(format!(
                "{} / `{DEP_STEP}` does not pass {CACHE_ENV}, so every run \
                 re-downloads the .deb archive:\n{body}",
                display(workflow)
            ));
        }
        if !body.contains(GTK_DEPS_MODE) {
            failures.push(format!(
                "{} / `{DEP_STEP}` must install through `{CI_APT} \
                 {GTK_DEPS_MODE}` so the package set has one home and the \
                 cache key can be derived from it:\n{body}",
                display(workflow)
            ));
        }

        let cache_step = named_steps(workflow_text)
            .into_iter()
            .find(|(name, _)| name == CACHE_STEP);
        let Some((_, cache_body)) = cache_step else {
            failures.push(format!(
                "{} installs the GTK dependencies but has no `{CACHE_STEP}` \
                 step, so AC-2 cannot hold",
                display(workflow)
            ));
            continue;
        };
        if !cache_body.contains("actions/cache@") {
            failures.push(format!(
                "{} / `{CACHE_STEP}` must use actions/cache:\n{cache_body}",
                display(workflow)
            ));
        }
        if !cache_body.contains(CI_APT) {
            failures.push(format!(
                "{} / `{CACHE_STEP}` must key the cache on {CI_APT}, which \
                 owns the package list; otherwise a changed package set is \
                 served the previous archive:\n{cache_body}",
                display(workflow)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the Linux dependency install must be cached:\n{}",
        failures.join("\n\n")
    );
}

#[cfg(unix)]
mod wrapper {
    use super::*;

    use std::os::unix::fs::PermissionsExt;
    use std::process::Output;

    use gwt_core::process::{resolved_command, ProcessPlanRequest};

    struct Harness {
        dir: PathBuf,
    }

    impl Harness {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gwt-4191-{name}-{}", std::process::id()));
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
            // No dpkg lock in the harness; the probe is the only lock source.
            command.env("GWT_APT_LOCK_PROBE", self.path("no-lock"));
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

    /// Records every apt-get invocation and fails until the recorded attempt
    /// count reaches `succeed_on`.
    fn flaky_apt_get(succeed_on: usize) -> String {
        format!(
            r#"#!/usr/bin/env bash
mkdir -p "${{GWT_APT_STATE_DIR}}"
printf '%s\n' "$*" >> "${{GWT_APT_STATE_DIR}}/argv"
count="${{GWT_APT_STATE_DIR}}/installs"
if [[ "$*" == *install* ]]; then
  printf 'x' >> "${{count}}"
  n=$(wc -c < "${{count}}" | tr -d ' ')
  if [[ "${{n}}" -lt {succeed_on} ]]; then
    echo "Temporary failure resolving 'azure.archive.ubuntu.com'" >&2
    exit 100
  fi
fi
exit 0
"#
        )
    }

    fn no_lock_probe() -> &'static str {
        "#!/usr/bin/env bash\nexit 1\n"
    }

    /// AC-4: a dependency install that fails for a network reason is retried
    /// rather than erasing the workspace test run.
    #[test]
    fn a_failed_dependency_install_is_retried_until_it_succeeds() {
        let harness = Harness::new("retry");
        harness.write_executable("no-lock", no_lock_probe());
        let apt_get = harness.write_executable("fake-apt-get", &flaky_apt_get(3));

        let output = harness.run(
            &[GTK_DEPS_MODE, "xvfb"],
            &[
                ("GWT_APT_GET", apt_get.to_string_lossy().as_ref()),
                ("GWT_APT_RETRY_DELAY", "0"),
            ],
        );
        let log = combined(&output);
        assert!(
            output.status.success(),
            "a transient dependency failure must be retried, not fatal:\n{log}"
        );

        let argv = fs::read_to_string(harness.path("state/argv")).expect("read argv");
        let installs = argv.lines().filter(|line| line.contains("install")).count();
        assert_eq!(
            installs, 3,
            "the install must be retried until it succeeds:\n{argv}\n{log}"
        );
        assert!(
            log.contains("attempt=1/") && log.contains("attempt=3/"),
            "every attempt must be logged with its ordinal so a run that only \
             passed on retry is visibly different from a healthy one:\n{log}"
        );

        let last = argv.lines().last().expect("at least one apt-get call");
        for package in [
            "libgtk-3-dev",
            "libwebkit2gtk-4.1-dev",
            "libayatana-appindicator3-dev",
            "libxdo-dev",
            "xvfb",
        ] {
            assert!(
                last.contains(package),
                "`{GTK_DEPS_MODE} xvfb` must install {package}:\n{last}"
            );
        }
    }

    /// AC-3: when the retry budget is spent the failure is reported as a
    /// dependency-install failure, in the log and in the job summary, so a
    /// job that never ran a test is not read as a test failure.
    #[test]
    fn an_exhausted_retry_budget_is_reported_as_a_dependency_install_failure() {
        let harness = Harness::new("exhausted");
        harness.write_executable("no-lock", no_lock_probe());
        let apt_get = harness.write_executable("fake-apt-get", &flaky_apt_get(99));
        let summary = harness.path("step-summary.md");
        fs::write(&summary, "").expect("seed step summary");

        let output = harness.run(
            &[GTK_DEPS_MODE],
            &[
                ("GWT_APT_GET", apt_get.to_string_lossy().as_ref()),
                ("GWT_APT_RETRY_DELAY", "0"),
                ("GITHUB_STEP_SUMMARY", summary.to_string_lossy().as_ref()),
            ],
        );
        let log = combined(&output);
        assert!(
            !output.status.success(),
            "an exhausted retry budget must fail the step:\n{log}"
        );
        assert!(
            log.contains("::error title=Linux dependency install failed::"),
            "the failure must be annotated as a dependency install failure:\n{log}"
        );

        let summary_text = fs::read_to_string(&summary).expect("read step summary");
        assert!(
            summary_text.contains("Linux dependency install failed"),
            "the job summary must name the dependency install:\n{summary_text}"
        );
        assert!(
            summary_text.contains("no tests were executed"),
            "the job summary must say no tests ran, which is what separates \
             this from a test failure:\n{summary_text}"
        );
    }

    /// AC-2: a handed-in cache directory is what apt downloads into, so a
    /// second run with a warm cache does not repeat the download.
    #[test]
    fn a_cache_directory_is_handed_to_apt_as_its_archive_dir() {
        let harness = Harness::new("cache");
        harness.write_executable("no-lock", no_lock_probe());
        let apt_get = harness.write_executable("fake-apt-get", &flaky_apt_get(1));
        let cache = harness.path("archives");

        let output = harness.run(
            &[GTK_DEPS_MODE],
            &[
                ("GWT_APT_GET", apt_get.to_string_lossy().as_ref()),
                (CACHE_ENV, cache.to_string_lossy().as_ref()),
            ],
        );
        let log = combined(&output);
        assert!(output.status.success(), "ci-apt must succeed:\n{log}");

        let argv = fs::read_to_string(harness.path("state/argv")).expect("read argv");
        let expected = format!("Dir::Cache::archives={}", cache.display());
        assert!(
            argv.contains(&expected),
            "apt-get must download into the cached archive dir:\n{argv}"
        );
        assert!(
            cache.join("partial").is_dir(),
            "the archive dir needs its partial/ subdirectory or apt refuses it"
        );
    }
}
