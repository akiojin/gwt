//! Issue #3609: mechanical guard for the process-global test isolation
//! contract inside the single `--bin gwt` test binary — which gwt home a test
//! owns, and how it takes the lock that protects it.
//!
//! `gwt_core::paths::gwt_home()` reads the process-global `HOME` on every
//! call, and it treats a `HOME` under `std::env::temp_dir()` as an explicit
//! isolated home (`crates/gwt-core/src/paths.rs`). Hundreds of tests in this
//! binary repoint `HOME` at their own `TempDir`, so any test that resolves a
//! gwt-home-derived path *without* pinning the home first can have its fixture
//! land inside a parallel test's tempdir — and vanish when that tempdir is
//! dropped. `load_issue_monitor_prefs_unlocked` answers a missing file with
//! `IssueMonitorPrefs::default()`, so the victim reloads silently empty and
//! panics on the first index access.
//!
//! Issues #3411, #3414 and #3601 were three single-test fixes for that exact
//! mechanism. This contract test is the detection mechanism that keeps the
//! fourth from happening: it fails the build instead of waiting for a CI
//! flake.
//!
//! Two guards are accepted, and they are not interchangeable:
//!
//! * `ScopedGwtHome::set(..)` pins a **thread-local** override that
//!   `gwt_home()` honours before it looks at `HOME`. It only covers work done
//!   on the test's own thread.
//! * `env_test_lock()` / `env_lock()` serialize every `HOME` mutation in the
//!   process. It covers the test thread *and* any worker thread it spawns,
//!   because those threads read the same process-global `HOME` the lock
//!   holder installed.
//!
//! Tests that hand path resolution to a spawned worker therefore need the
//! lock; the thread-local pin is not visible from the worker thread. See
//! [`SCAN_WORKER_TRIGGERS`].
//!
//! Scope: this test covers the `--bin gwt` test tree, the one test binary all
//! three historical flakes occurred in. Other test binaries are separate
//! processes and do not share this `HOME`; several of them pin the home from
//! a fixture struct rather than the test body, which this simple body-level
//! detector cannot see.
//!
//! Known gap, stated so nobody reads a pass here as full coverage: the rules
//! below follow calls a test makes *directly*, plus fixtures declared in
//! [`SHARED_FIXTURE_SOURCE`]. They do not follow a test into production code.
//! A test that drives, say, `handle_frontend_event(OpenIntakeSession)` reaches
//! `reserve_start_work_branch_name_for_project`, which resolves
//! `gwt_project_dir_for_repo_path` two hops away — and this detector stays
//! silent. Name-based propagation through production code was measured and
//! rejected: it reaches generic entry points such as `handle_frontend_event`
//! and `new`, and would flag roughly 260 tests without distinguishing the ones
//! that actually resolve a home. Closing that class needs either per-test home
//! pinning across the binary or a runtime guard inside `gwt_home()`, not a
//! wider static rule.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Path helpers that resolve the gwt home from process-global state instead of
/// from an argument. Calling one of these decides *where* a fixture lives, so
/// the caller must have pinned the home first.
const HOME_IMPLICIT_PATH_HELPERS: &[&str] = &[
    "gwt_cache_dir",
    "gwt_config_path",
    "gwt_coordination_root",
    "gwt_home",
    "gwt_logs_dir",
    "gwt_notes_dir",
    "gwt_project_dir_for_repo_path",
    "gwt_projects_dir",
    "gwt_runtime_dir",
    "gwt_session_state_path",
    "gwt_sessions_dir",
    "gwt_updates_dir",
    "gwt_workspace_projection_path_for_repo_path",
    "issue_monitor_prefs_path_for_repo_path",
    "pm_prefs_path_for_repo_path",
    "workspace_state_path",
];

/// Files that must define [`HOME_IMPLICIT_PATH_HELPERS`], so a typo in that
/// list fails loudly instead of silently disabling a rule.
const HOME_IMPLICIT_PATH_HELPER_SOURCES: &[&str] = &[
    "crates/gwt-core/src/paths.rs",
    "crates/gwt/src/issue_monitor.rs",
    "crates/gwt/src/persistence.rs",
    "crates/gwt/src/pm_registry.rs",
];

/// The shared test file that owns the `--bin gwt` fixtures. Fixtures declared
/// here resolve gwt-home paths on behalf of their callers, so the guard
/// obligation propagates from them to every test that uses them.
const SHARED_FIXTURE_SOURCE: &str = "crates/gwt/src/app_runtime/tests.rs";

/// Test entry points that enqueue the Issue Monitor scheduled scan worker.
/// `run_scheduled_issue_monitor_scan_with_budgets` re-resolves the prefs path
/// from `project_root` **on the worker thread**, where a `ScopedGwtHome`
/// thread-local pin installed by the test body is not visible. These tests
/// must serialize the process-global `HOME` instead.
const SCAN_WORKER_TRIGGERS: &[&str] = &[
    "authenticated_issue_monitor_scan_now_events",
    "issue_monitor_scheduled_tick_events_at",
    "wait_for_scheduled_scan_completion",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

/// One function definition, reduced to what the rules need.
#[derive(Debug, Clone)]
struct ParsedFn {
    name: String,
    line: usize,
    is_test: bool,
    calls: BTreeSet<String>,
    holds_env_lock: bool,
    pins_thread_local_home: bool,
}

impl ParsedFn {
    fn guarded(&self) -> bool {
        self.holds_env_lock || self.pins_thread_local_home
    }
}

/// Splits `source` into top-of-block function definitions.
///
/// The repository is `cargo fmt`-formatted, so a function body always ends at
/// the first line that is exactly the signature's indentation followed by
/// `}`. That is far more robust here than brace counting, which a 50k-line
/// test file full of `"{}"` format literals would defeat.
fn parse_functions(source: &str) -> Vec<ParsedFn> {
    let lines: Vec<&str> = source.lines().collect();
    let mut parsed = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some((indent, name)) = function_signature(line) else {
            continue;
        };
        let closer = format!("{indent}}}");
        let end = lines[index + 1..]
            .iter()
            .position(|candidate| *candidate == closer)
            .map(|offset| index + 1 + offset)
            .unwrap_or(lines.len() - 1);
        let body = lines[index..=end].join("\n");
        parsed.push(ParsedFn {
            name: name.to_string(),
            line: index + 1,
            is_test: has_test_attribute(&lines[..index]),
            calls: called_names(&body),
            holds_env_lock: acquires_env_lock(&body),
            pins_thread_local_home: body.contains("ScopedGwtHome::set"),
        });
    }
    parsed
}

/// Returns `(indent, name)` when `line` opens a function definition.
fn function_signature(line: &str) -> Option<(&str, &str)> {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, mut rest) = line.split_at(indent_len);
    for prefix in ["pub(crate) ", "pub(super) ", "pub(self) ", "pub ", "async "] {
        rest = rest.strip_prefix(prefix).unwrap_or(rest);
    }
    let rest = rest.strip_prefix("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    let name_len = name.len();
    Some((indent, &rest[..name_len]))
}

/// True when the attribute block directly above a function marks it a test.
fn has_test_attribute(preceding: &[&str]) -> bool {
    for line in preceding.iter().rev() {
        let trimmed = line.trim();
        if trimmed == "#[test]" || trimmed.ends_with("::test]") {
            return true;
        }
        if trimmed.is_empty() || trimmed.starts_with("#[") || trimmed.starts_with("//") {
            continue;
        }
        return false;
    }
    false
}

/// Every identifier immediately followed by `(`. Path-qualified calls such as
/// `paths::gwt_cache_dir(..)` yield their final segment, which is what the
/// rule lists name.
fn called_names(body: &str) -> BTreeSet<String> {
    let bytes = body.as_bytes();
    let mut names = BTreeSet::new();
    let mut start = None;
    for (index, byte) in bytes.iter().enumerate() {
        let is_ident = byte.is_ascii_alphanumeric() || *byte == b'_';
        if is_ident {
            start.get_or_insert(index);
            continue;
        }
        if let Some(begin) = start.take() {
            if *byte == b'(' {
                names.insert(body[begin..index].to_string());
            }
        }
    }
    names
}

/// True when the body locks the process-wide environment mutex, allowing
/// whitespace and line breaks between the accessor and `.lock()`.
fn acquires_env_lock(body: &str) -> bool {
    ["env_test_lock()", "env_lock()"]
        .iter()
        .any(|accessor| lock_call_follows(body, accessor))
}

fn lock_call_follows(body: &str, accessor: &str) -> bool {
    let mut rest = body;
    while let Some(offset) = rest.find(accessor) {
        let after = rest[offset + accessor.len()..].trim_start();
        if after.starts_with(".lock()") {
            return true;
        }
        rest = &rest[offset + accessor.len()..];
    }
    false
}

/// Source files compiled into the `--bin gwt` test binary, derived from the
/// binary's own `mod` declarations so a new module cannot escape the rules.
fn bin_gwt_test_sources(root: &Path) -> Vec<PathBuf> {
    let main_path = root.join("crates/gwt/src/main.rs");
    let main_source = read_source(&main_path);
    let src_root = root.join("crates/gwt/src");
    let mut sources = vec![main_path];
    for line in main_source.lines() {
        let Some(name) = line
            .strip_prefix("mod ")
            .and_then(|rest| rest.strip_suffix(';'))
        else {
            continue;
        };
        let flat = src_root.join(format!("{name}.rs"));
        if flat.is_file() {
            sources.push(flat);
        }
        let directory = src_root.join(name);
        if directory.is_dir() {
            collect_rust_sources(&directory, &mut sources);
        }
    }
    sources.sort();
    sources
}

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

fn read_source(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

/// Fixtures in [`SHARED_FIXTURE_SOURCE`] that reach a home-implicit helper,
/// directly or through another fixture in the same file.
fn derived_home_implicit_fixtures(fixture_source: &str) -> BTreeSet<String> {
    let watched: BTreeSet<String> = HOME_IMPLICIT_PATH_HELPERS
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let fixtures: Vec<ParsedFn> = parse_functions(fixture_source)
        .into_iter()
        .filter(|function| !function.is_test)
        .collect();
    let mut closure: BTreeSet<String> = BTreeSet::new();
    loop {
        let mut grew = false;
        for fixture in &fixtures {
            if closure.contains(&fixture.name) {
                continue;
            }
            let reaches = fixture
                .calls
                .iter()
                .any(|call| watched.contains(call) || closure.contains(call));
            if reaches {
                closure.insert(fixture.name.clone());
                grew = true;
            }
        }
        if !grew {
            return closure;
        }
    }
}

struct Violation {
    file: String,
    line: usize,
    function: String,
    reached: Vec<String>,
}

fn report(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|violation| {
            format!(
                "  {}:{} {} -> {}",
                violation.file,
                violation.line,
                violation.function,
                violation.reached.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn violations_for(
    root: &Path,
    watched: &BTreeSet<String>,
    accept: impl Fn(&ParsedFn) -> bool,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for path in bin_gwt_test_sources(root) {
        let source = read_source(&path);
        for function in parse_functions(&source) {
            if !function.is_test {
                continue;
            }
            let reached: Vec<String> = function
                .calls
                .iter()
                .filter(|call| watched.contains(*call))
                .cloned()
                .collect();
            if reached.is_empty() || accept(&function) {
                continue;
            }
            violations.push(Violation {
                file: relative(root, &path),
                line: function.line,
                function: function.name.clone(),
                reached,
            });
        }
    }
    violations
}

/// Every `env_lock()` / `env_test_lock()` acquisition, with the expression
/// that consumes the resulting `LockResult`.
fn env_lock_acquisitions(source: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut acquisitions = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.contains("env_lock()") && !line.contains("env_test_lock()") {
            continue;
        }
        let window = lines[index..lines.len().min(index + 4)].join(" ");
        let Some(offset) = window.find(".lock()") else {
            continue;
        };
        let consumer = &window[offset..];
        let consumer = consumer.split_once(';').map_or(consumer, |(head, _)| head);
        acquisitions.push((
            index + 1,
            consumer.split_whitespace().collect::<Vec<_>>().join(" "),
        ));
    }
    acquisitions
}

#[test]
fn every_env_lock_acquisition_recovers_from_poisoning() {
    let root = repo_root();
    let mut violations = Vec::new();
    for path in bin_gwt_test_sources(&root) {
        let source = read_source(&path);
        for (line, consumer) in env_lock_acquisitions(&source) {
            // `unwrap_or_else(PoisonError::into_inner)` and the equivalent
            // closure form both recover; anything else aborts the test.
            if consumer.contains("into_inner") {
                continue;
            }
            violations.push(format!("  {}:{line} {consumer}", relative(&root, &path)));
        }
    }
    assert!(
        violations.is_empty(),
        "Issue #3609: {} env-lock acquisition(s) abort on a poisoned mutex. One panicking \
         test then fails every later lock holder, turning a single real failure into a \
         cascade that hides its own cause. Use \
         `.lock().unwrap_or_else(std::sync::PoisonError::into_inner)`:\n{}",
        violations.len(),
        violations.join("\n")
    );
}

#[test]
fn poison_recovery_rule_reads_the_consuming_expression() {
    let source = "\
fn recovers() {
    let _guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
}

fn recovers_with_a_closure() {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
}

fn aborts() {
    let _guard = crate::env_test_lock().lock().expect(\"env lock\");
}

fn mentions_lock_without_acquiring() {
    // env_lock() is documented here but never taken.
}
";
    let acquisitions = env_lock_acquisitions(source);
    assert_eq!(acquisitions.len(), 3, "only real acquisitions count");
    assert!(acquisitions[0].1.contains("PoisonError::into_inner"));
    assert!(acquisitions[1].1.contains("poisoned.into_inner()"));
    assert!(!acquisitions[2].1.contains("into_inner"));
}

#[test]
fn every_home_implicit_test_pins_its_gwt_home() {
    let root = repo_root();
    let fixture_source = read_source(&root.join(SHARED_FIXTURE_SOURCE));
    let watched: BTreeSet<String> = HOME_IMPLICIT_PATH_HELPERS
        .iter()
        .map(|name| (*name).to_string())
        .chain(derived_home_implicit_fixtures(&fixture_source))
        .collect();
    let violations = violations_for(&root, &watched, ParsedFn::guarded);
    assert!(
        violations.is_empty(),
        "Issue #3609: {} test(s) in the `--bin gwt` binary resolve a gwt-home path without \
         owning the home. Take `env_test_lock()` (and repoint `HOME`) or pin \
         `ScopedGwtHome::set(<test temp root>)` before the path is resolved, or make the \
         fixture derive the path from the test's own temp root:\n{}",
        violations.len(),
        report(&violations)
    );
}

#[test]
fn every_scan_worker_test_holds_the_process_env_lock() {
    let root = repo_root();
    let watched: BTreeSet<String> = SCAN_WORKER_TRIGGERS
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let violations = violations_for(&root, &watched, |function| function.holds_env_lock);
    assert!(
        violations.is_empty(),
        "Issue #3609: {} test(s) enqueue the Issue Monitor scan worker without holding \
         `env_test_lock()`. The worker re-resolves the prefs path from the process-global \
         `HOME` on its own thread, where a `ScopedGwtHome` thread-local pin is invisible:\n{}",
        violations.len(),
        report(&violations)
    );
}

#[test]
fn home_implicit_path_helpers_are_real_functions() {
    let root = repo_root();
    let sources: Vec<String> = HOME_IMPLICIT_PATH_HELPER_SOURCES
        .iter()
        .map(|path| read_source(&root.join(path)))
        .collect();
    for helper in HOME_IMPLICIT_PATH_HELPERS {
        let marker = format!("pub fn {helper}(");
        assert!(
            sources.iter().any(|source| source.contains(&marker)),
            "`{helper}` is listed as a home-implicit path helper but is not defined in {:?}",
            HOME_IMPLICIT_PATH_HELPER_SOURCES
        );
    }
}

#[test]
fn bin_gwt_test_sources_cover_the_shared_fixture_file() {
    let root = repo_root();
    let sources = bin_gwt_test_sources(&root);
    let relative_paths: BTreeSet<String> =
        sources.iter().map(|path| relative(&root, path)).collect();
    assert!(
        relative_paths.contains(SHARED_FIXTURE_SOURCE),
        "the `--bin gwt` source walk must reach {SHARED_FIXTURE_SOURCE}, found {relative_paths:?}"
    );
    assert!(
        relative_paths.contains("crates/gwt/src/main.rs"),
        "the `--bin gwt` source walk must include the binary root"
    );
}

#[test]
fn parse_functions_detects_tests_calls_and_guards() {
    let source = "\
#[test]
fn unguarded() {
    let path = gwt_cache_dir();
}

#[test]
fn locked() {
    let _guard = env_test_lock()
        .lock()
        .unwrap();
    let path = gwt_cache_dir();
}

#[test]
fn pinned() {
    let _home = ScopedGwtHome::set(temp.path());
    let path = gwt_cache_dir();
}

fn helper() {
    let path = gwt_cache_dir();
}
";
    let parsed = parse_functions(source);
    let names: Vec<&str> = parsed
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(names, vec!["unguarded", "locked", "pinned", "helper"]);

    let unguarded = &parsed[0];
    assert!(unguarded.is_test);
    assert!(unguarded.calls.contains("gwt_cache_dir"));
    assert!(!unguarded.guarded());

    assert!(parsed[1].holds_env_lock, "a multi-line .lock() counts");
    assert!(parsed[2].pins_thread_local_home);
    assert!(!parsed[3].is_test, "a plain helper is not a test");
}

#[test]
fn env_lock_mention_without_a_lock_call_is_not_a_guard() {
    let source = "\
#[test]
fn mentions_only() {
    // env_test_lock() is deliberately not taken here.
    let path = gwt_cache_dir();
}
";
    let parsed = parse_functions(source);
    assert!(!parsed[0].holds_env_lock);
    assert!(!parsed[0].guarded());
}

#[test]
fn derived_fixtures_propagate_through_wrapper_fixtures() {
    let source = "\
fn leaf_fixture() {
    let path = gwt_cache_dir();
}

fn wrapper_fixture() {
    leaf_fixture();
}

fn unrelated_fixture() {
    let path = temp_root.join(\"cache\");
}

#[test]
fn a_test_is_never_a_fixture() {
    let path = gwt_cache_dir();
}
";
    let derived = derived_home_implicit_fixtures(source);
    assert_eq!(
        derived,
        BTreeSet::from(["leaf_fixture".to_string(), "wrapper_fixture".to_string()])
    );
}
