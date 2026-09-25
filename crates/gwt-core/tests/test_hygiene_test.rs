//! Test-hygiene gates (SPEC #4551).
//!
//! These tests scan the workspace's own test code and fail when a known
//! flake mechanism is written. They exist because fixing flakes one at a
//! time did not work: 16 open issues accumulated over three and a half
//! months, every one of them discovered *after* CI went red.
//!
//! The following mechanisms are gated here:
//!
//! * **(A) wall-clock dependence** — a sub-100ms `Duration` literal in test
//!   code. Scheduling jitter on a saturated runner is measured in tens of
//!   milliseconds, so an assertion that assumes ordering inside that window
//!   cannot be made deterministic.
//!   Native index watcher delivery also has no clock bound, even at 30 seconds
//!   (Issue #4681). Direct start_watcher/recv_batch/timeout combinations are
//!   gated separately; retained real-OS integration probes need a reason.
//! * **(B) unlocked process-wide state** — `env::set_var` / `remove_var` /
//!   `set_current_dir` without holding the shared env lock. Those are
//!   process-global and stomp on sibling tests in the same binary.
//! * **(C) production probe deadlines** — test calls to
//!   `ResolvedContainerRuntime::resolve` start a real process under an internal
//!   deadline, even when the test contains no Duration literal. Retained
//!   process integration contracts need a reason; pure tests inject the probe.
//!   This lexical rule covers that known entrypoint, not aliases or indirect
//!   calls through arbitrary helpers.
//!
//! Existing violations are grandfathered in `test_hygiene_baseline.txt`, one
//! line per `<rule>|<file>|<context>`. The baseline only shrinks: a violation
//! in a context that is not listed fails, and a listed context that no longer
//! violates fails too. New, genuinely justified exceptions are declared
//! inline instead, on the offending line or the line above it:
//!
//! ```text
//! // test-hygiene: allow-short-duration polling interval, not an assertion deadline
//! // test-hygiene: allow-unlocked-env the whole binary is single-threaded
//! // test-hygiene: allow-native-watcher-deadline retained real-OS integration probe
//! // test-hygiene: allow-production-probe-deadline retained wrapper execution contract
//! ```
//!
//! The reason text is mandatory; a bare marker does not silence the gate.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use hygiene::{scan_source, target_sources, Rule, SourceKind};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize workspace root")
}

fn baseline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/test_hygiene_baseline.txt")
}

fn read_baseline() -> BTreeSet<String> {
    let text = fs::read_to_string(baseline_path()).expect("read test hygiene baseline");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn scan_workspace() -> Vec<hygiene::Finding> {
    let root = workspace_root();
    let mut findings = Vec::new();
    for source in target_sources(&root) {
        let text = match fs::read_to_string(root.join(&source.relative_path)) {
            Ok(text) => text,
            Err(error) => panic!("read {}: {error}", source.relative_path),
        };
        findings.extend(scan_source(&source.relative_path, &text, source.kind));
    }
    findings
}

// ---------------------------------------------------------------------------
// T-010: the scan itself has to reach real files before any rule matters.
// ---------------------------------------------------------------------------

#[test]
fn workspace_scan_reaches_integration_and_unit_test_sources() {
    let root = workspace_root();
    let sources = target_sources(&root);

    assert!(
        sources.len() > 100,
        "expected the workspace scan to enumerate many sources, got {}",
        sources.len()
    );

    let integration = sources
        .iter()
        .find(|source| source.relative_path == "crates/gwt-core/tests/test_hygiene_test.rs")
        .expect("this very test file must be part of the scanned corpus");
    assert_eq!(integration.kind, SourceKind::WholeFileIsTest);

    let unit = sources
        .iter()
        .find(|source| source.relative_path == "crates/gwt-core/src/test_support.rs")
        .expect("crate sources must be part of the scanned corpus");
    assert_eq!(unit.kind, SourceKind::CfgTestBlocksOnly);

    for source in &sources {
        assert!(
            root.join(&source.relative_path).is_file(),
            "enumerated source does not exist: {}",
            source.relative_path
        );
    }
}

/// A unit-test module declared as `#[cfg(test)] mod tests;` lives in its own
/// file, and nothing inside that file carries the attribute. Classifying it by
/// its contents therefore made the whole file unscannable, which is how
/// `crates/gwt/src/app_runtime/tests.rs` kept a `Duration::from_millis(25)`
/// with a green gate and no baseline row. The workspace has 15 such modules,
/// and several of the issues SPEC #4551 bundles live in them — which is exactly
/// the code the gate exists to cover.
#[test]
fn an_externally_declared_cfg_test_module_is_scanned_as_whole_file_test_code() {
    let root = workspace_root();
    let sources = target_sources(&root);

    let app_runtime = sources
        .iter()
        .find(|source| source.relative_path == "crates/gwt/src/app_runtime/tests.rs")
        .expect("`#[cfg(test)] mod tests;` files must be part of the scanned corpus");
    assert_eq!(
        app_runtime.kind,
        SourceKind::WholeFileIsTest,
        "a file that exists only because a `#[cfg(test)] mod` declaration names it \
         is test code from its first line"
    );

    // A production module sitting beside it must keep the narrow treatment.
    let production = sources
        .iter()
        .find(|source| source.relative_path == "crates/gwt/src/app_runtime/mod.rs")
        .expect("the declaring module is still a crate source");
    assert_eq!(production.kind, SourceKind::CfgTestBlocksOnly);
}

// ---------------------------------------------------------------------------
// Scanner unit tests — fixtures, so the rules themselves are covered without
// depending on whatever the workspace happens to contain today.
// ---------------------------------------------------------------------------

fn scan_fixture(text: &str) -> Vec<hygiene::Finding> {
    scan_source(
        "crates/demo/tests/demo.rs",
        text,
        SourceKind::WholeFileIsTest,
    )
}

#[test]
fn production_probe_deadline_is_reported_without_a_duration_literal() {
    let findings = scan_fixture(
        r#"
fn resolves_fake_runtime() {
    gwt_docker::detect::ResolvedContainerRuntime::resolve(binary).unwrap();
}
fn seconds_do_not_make_a_real_probe_deterministic() {
    let timeout = Duration::from_secs(30);
    ResolvedContainerRuntime::resolve(binary).unwrap();
}
"#,
    );
    assert_eq!(findings.len(), 2, "unexpected findings: {findings:?}");
    assert!(findings
        .iter()
        .all(|finding| finding.rule.id() == "production-probe-deadline"));
}

#[test]
fn production_probe_deadline_requires_a_reason_for_real_process_contracts() {
    let findings = scan_fixture(
        r#"
fn declared_process_contract() {
    // test-hygiene: allow-production-probe-deadline Verify the actual wrapper is probed once.
    ResolvedContainerRuntime::resolve(binary).unwrap();
}
fn missing_reason() {
    // test-hygiene: allow-production-probe-deadline
    ResolvedContainerRuntime::resolve(binary).unwrap();
}
"#,
    );
    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].context, "missing_reason");
}

#[test]
fn production_probe_deadline_ignores_production_code_and_injected_probe_seams() {
    let findings = scan_source(
        "crates/demo/src/lib.rs",
        r#"
fn production() {
    ResolvedContainerRuntime::resolve(binary).unwrap();
}
#[cfg(test)]
mod tests {
    fn injected_probe() {
        ResolvedContainerRuntime::resolve_with_probe(binary, probe).unwrap();
        // ResolvedContainerRuntime::resolve(binary)
        let source = "ResolvedContainerRuntime::resolve(binary)";
    }
}
"#,
        SourceKind::CfgTestBlocksOnly,
    );
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn native_watcher_delivery_deadlines_are_reported_regardless_of_duration() {
    let findings = scan_fixture(
        r#"
async fn waits_for_native_delivery() {
    let mut watcher = start_watcher(root, config).unwrap();
    tokio::time::timeout(Duration::from_secs(30), watcher.recv_batch()).await.unwrap();
}

"#,
    );
    assert_eq!(
        findings.len(),
        1,
        "native delivery is not bounded by a clock"
    );
    assert_eq!(findings[0].rule.id(), "native-watcher-deadline");
    assert_eq!(findings[0].context, "waits_for_native_delivery");
}

#[test]
fn native_watcher_startup_and_declared_os_probes_are_allowed() {
    let findings = scan_fixture(
        r#"
async fn startup_only() {
    let watcher = start_watcher(root, config).unwrap();
    watcher.shutdown().await;
}
async fn explicit_os_probe() {
    // test-hygiene: allow-native-watcher-deadline Real OS integration probe.
    let mut watcher = start_watcher(root, config).unwrap();
    tokio::time::timeout(EVENT_TIMEOUT, watcher.recv_batch()).await.unwrap();
}
"#,
    );
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}
#[test]
fn short_wall_clock_durations_are_reported_with_their_enclosing_function() {
    let findings = scan_fixture(
        r#"
#[test]
fn waits_too_briefly() {
    std::thread::sleep(Duration::from_millis(5));
}
"#,
    );

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].rule, Rule::ShortWallClock);
    assert_eq!(findings[0].context, "waits_too_briefly");
    assert_eq!(findings[0].line, 4);
}

#[test]
fn durations_at_or_above_the_threshold_are_accepted() {
    let findings = scan_fixture(
        r#"
fn waits_long_enough() {
    std::thread::sleep(Duration::from_millis(100));
    std::thread::sleep(std::time::Duration::from_secs(1));
    let _ = Duration::from_millis(1_500);
}
"#,
    );

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn sub_millisecond_constructors_are_reported_regardless_of_value() {
    let findings = scan_fixture(
        r#"
fn spins() {
    std::thread::sleep(Duration::from_micros(900));
    std::thread::sleep(Duration::from_nanos(1));
}
"#,
    );

    assert_eq!(findings.len(), 2, "unexpected findings: {findings:?}");
    assert!(findings.iter().all(|f| f.rule == Rule::ShortWallClock));
}

#[test]
fn non_literal_durations_are_not_guessed_at() {
    let findings = scan_fixture(
        r#"
fn uses_a_constant() {
    std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
}
"#,
    );

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn durations_inside_strings_and_comments_are_ignored() {
    let findings = scan_fixture(
        r##"
fn inspects_text() {
    // Duration::from_millis(1)
    let rendered = "Duration::from_millis(1)";
    let raw = r#"Duration::from_millis(2)"#;
    let _ = (rendered, raw);
    /* Duration::from_millis(3) */
}
"##,
    );

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn an_inline_allow_with_a_reason_silences_the_wall_clock_rule() {
    let above = scan_fixture(
        r#"
fn polls() {
    // test-hygiene: allow-short-duration polling interval, not an assertion deadline
    std::thread::sleep(Duration::from_millis(5));
}
"#,
    );
    assert!(above.is_empty(), "unexpected findings: {above:?}");

    let trailing = scan_fixture(
        r#"
fn polls() {
    std::thread::sleep(Duration::from_millis(5)); // test-hygiene: allow-short-duration polling interval
}
"#,
    );
    assert!(trailing.is_empty(), "unexpected findings: {trailing:?}");
}

#[test]
fn an_inline_allow_without_a_reason_does_not_silence_the_rule() {
    let findings = scan_fixture(
        r#"
fn polls() {
    // test-hygiene: allow-short-duration
    std::thread::sleep(Duration::from_millis(5));
}
"#,
    );

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
}

#[test]
fn process_env_mutation_without_the_shared_lock_is_reported() {
    let findings = scan_fixture(
        r#"
#[test]
fn rewrites_path() {
    std::env::set_var("PATH", "/tmp");
    env::remove_var("HOME");
    std::env::set_current_dir("/tmp").unwrap();
}
"#,
    );

    assert_eq!(findings.len(), 3, "unexpected findings: {findings:?}");
    assert!(findings.iter().all(|f| f.rule == Rule::UnlockedProcessEnv));
    assert!(findings.iter().all(|f| f.context == "rewrites_path"));
}

#[test]
fn process_env_mutation_under_a_shared_lock_is_accepted() {
    let guarded = scan_fixture(
        r#"
#[test]
fn rewrites_path() {
    let _guard = env_test_lock().lock().unwrap();
    std::env::set_var("PATH", "/tmp");
}
"#,
    );
    assert!(guarded.is_empty(), "unexpected findings: {guarded:?}");

    let scoped = scan_fixture(
        r#"
#[test]
fn rewrites_path() {
    let _guard = gwt_core::test_support::env_lock().lock().unwrap();
    std::env::remove_var("HOME");
}
"#,
    );
    assert!(scoped.is_empty(), "unexpected findings: {scoped:?}");
}

#[test]
fn the_env_lock_must_be_held_by_the_same_function() {
    let findings = scan_fixture(
        r#"
fn helper() {
    let _guard = env_test_lock().lock().unwrap();
}

#[test]
fn rewrites_path() {
    helper();
    std::env::set_var("PATH", "/tmp");
}
"#,
    );

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].context, "rewrites_path");
}

#[test]
fn an_inline_allow_with_a_reason_silences_the_env_rule() {
    let findings = scan_fixture(
        r#"
#[test]
fn rewrites_path() {
    // test-hygiene: allow-unlocked-env this binary runs a single test
    std::env::set_var("PATH", "/tmp");
}
"#,
    );

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn only_cfg_test_blocks_are_scanned_in_crate_sources() {
    let text = r#"
pub fn production() {
    std::thread::sleep(Duration::from_millis(5));
}

#[cfg(test)]
mod tests {
    #[test]
    fn in_a_test() {
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn more_production() {
    std::env::set_var("PATH", "/tmp");
}
"#;

    let findings = scan_source(
        "crates/demo/src/lib.rs",
        text,
        SourceKind::CfgTestBlocksOnly,
    );
    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].context, "in_a_test");
}

#[test]
fn a_cfg_test_function_inside_an_impl_block_is_scanned() {
    let text = r#"
impl Fixture {
    #[cfg(test)]
    fn probe(&self) {
        std::thread::sleep(Duration::from_millis(5));
    }

    fn ship(&self) {
        std::thread::sleep(Duration::from_millis(5));
    }
}
"#;

    let findings = scan_source(
        "crates/demo/src/lib.rs",
        text,
        SourceKind::CfgTestBlocksOnly,
    );
    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].context, "probe");
}

// ---------------------------------------------------------------------------
// T-020 / AC-1 and T-040 / AC-3: the workspace gates.
// ---------------------------------------------------------------------------

fn assert_no_new_violations(rule: Rule) {
    let baseline = read_baseline();
    let mut offenders: Vec<String> = scan_workspace()
        .into_iter()
        .filter(|finding| finding.rule == rule)
        .filter(|finding| !baseline.contains(&finding.baseline_key()))
        .map(|finding| {
            format!(
                "  {}:{} in `{}`\n      {}\n      baseline key: {}",
                finding.file,
                finding.line,
                finding.context,
                finding.snippet,
                finding.baseline_key()
            )
        })
        .collect();
    offenders.sort();
    offenders.dedup();

    assert!(
        offenders.is_empty(),
        "{} new {} violation(s).\n\n{}\n\n{}\n",
        offenders.len(),
        rule.id(),
        offenders.join("\n"),
        rule.remedy()
    );
}

#[test]
fn no_new_short_wall_clock_durations_in_test_code() {
    assert_no_new_violations(Rule::ShortWallClock);
}

#[test]
fn no_new_unlocked_process_env_mutations_in_test_code() {
    assert_no_new_violations(Rule::UnlockedProcessEnv);
}

#[test]
fn no_undeclared_native_watcher_delivery_deadlines() {
    assert_no_new_violations(Rule::NativeWatcherDeadline);
}

#[test]
fn no_undeclared_production_probe_deadlines() {
    assert_no_new_violations(Rule::ProductionProbeDeadline);
}

#[test]
fn the_baseline_only_shrinks() {
    let baseline = read_baseline();
    let live: BTreeSet<String> = scan_workspace()
        .iter()
        .map(hygiene::Finding::baseline_key)
        .collect();

    let stale: Vec<&String> = baseline.difference(&live).collect();
    assert!(
        stale.is_empty(),
        "{} baseline entr(ies) no longer match a violation. Delete these lines from \
         crates/gwt-core/tests/test_hygiene_baseline.txt:\n{}\n",
        stale.len(),
        stale
            .iter()
            .map(|entry| format!("  {entry}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_baseline_is_well_formed_and_sorted() {
    let text = fs::read_to_string(baseline_path()).expect("read test hygiene baseline");
    let entries: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    for entry in &entries {
        let fields: Vec<&str> = entry.split('|').collect();
        assert_eq!(
            fields.len(),
            3,
            "baseline entry must be `<rule>|<file>|<context>`: {entry}"
        );
        assert!(
            Rule::from_id(fields[0]).is_some(),
            "unknown rule id in baseline entry: {entry}"
        );
        assert!(
            !fields[1].is_empty() && !fields[2].is_empty(),
            "baseline entry has an empty field: {entry}"
        );
    }

    let mut sorted = entries.clone();
    sorted.sort_unstable();
    assert_eq!(
        entries, sorted,
        "the baseline must stay sorted so concurrent edits do not collide"
    );

    let unique: BTreeSet<&&str> = entries.iter().collect();
    assert_eq!(
        unique.len(),
        entries.len(),
        "the baseline has duplicate rows"
    );
}

/// Maintenance helper. Rewrites the baseline from the current tree:
///
/// ```text
/// cargo test -p gwt-core --test test_hygiene_test -- --ignored regenerate_the_baseline
/// ```
///
/// Use it when a bulk cleanup lands; a single fix is a one-line deletion and
/// does not need it. Regenerating never hides a new violation from review —
/// the added rows show up in the diff.
#[test]
#[ignore = "maintenance helper: rewrites test_hygiene_baseline.txt"]
fn regenerate_the_baseline() {
    let keys: BTreeSet<String> = scan_workspace()
        .iter()
        .map(hygiene::Finding::baseline_key)
        .collect();

    let mut text = String::from(BASELINE_HEADER);
    for key in &keys {
        text.push_str(key);
        text.push('\n');
    }
    fs::write(baseline_path(), text).expect("write test hygiene baseline");
    eprintln!("wrote {} baseline entr(ies)", keys.len());
}

const BASELINE_HEADER: &str = "\
# Grandfathered test-hygiene violations (SPEC #4551).
#
# Format: <rule>|<file>|<context>
#
# This list only shrinks. Do not append to it: fix the violation, or declare
# the exception inline with a reason on the offending line
# (`// test-hygiene: allow-short-duration <reason>` /
# `// test-hygiene: allow-unlocked-env <reason>`).
#
# Entries are keyed by enclosing function rather than by line number, so
# unrelated edits above a violation do not churn this file. Once a listed
# context stops violating, its row must be deleted or the gate fails.
";

// ---------------------------------------------------------------------------
// The scanner.
// ---------------------------------------------------------------------------

mod hygiene {
    use super::{Path, PathBuf};
    use regex::Regex;
    use std::sync::OnceLock;

    /// Longest scheduling hiccup we are willing to call "noise" on a
    /// saturated CI runner. Anything shorter cannot carry an assertion.
    const SHORT_DURATION_THRESHOLD_MS: u64 = 100;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Rule {
        ShortWallClock,
        UnlockedProcessEnv,
        NativeWatcherDeadline,
        ProductionProbeDeadline,
    }

    impl Rule {
        pub fn id(self) -> &'static str {
            match self {
                Rule::ShortWallClock => "short-wall-clock",
                Rule::UnlockedProcessEnv => "unlocked-process-env",
                Rule::NativeWatcherDeadline => "native-watcher-deadline",
                Rule::ProductionProbeDeadline => "production-probe-deadline",
            }
        }

        pub fn from_id(id: &str) -> Option<Self> {
            match id {
                "short-wall-clock" => Some(Rule::ShortWallClock),
                "unlocked-process-env" => Some(Rule::UnlockedProcessEnv),
                "native-watcher-deadline" => Some(Rule::NativeWatcherDeadline),
                "production-probe-deadline" => Some(Rule::ProductionProbeDeadline),
                _ => None,
            }
        }

        fn allow_marker(self) -> &'static str {
            match self {
                Rule::ShortWallClock => "test-hygiene: allow-short-duration",
                Rule::UnlockedProcessEnv => "test-hygiene: allow-unlocked-env",
                Rule::NativeWatcherDeadline => "test-hygiene: allow-native-watcher-deadline",
                Rule::ProductionProbeDeadline => "test-hygiene: allow-production-probe-deadline",
            }
        }

        pub fn remedy(self) -> &'static str {
            match self {
                Rule::ShortWallClock => concat!(
                    "A sub-100ms Duration cannot order anything on a loaded runner.\n",
                    "Express the deadline instead of waiting for it: have the fake\n",
                    "transport observe the deadline elapse, or raise the value past\n",
                    "100ms when it is only a bound. If the value is genuinely not an\n",
                    "assertion premise (a polling interval, a synthetic Instant offset),\n",
                    "declare it on the line or the line above:\n",
                    "    // test-hygiene: allow-short-duration <reason>"
                ),
                Rule::UnlockedProcessEnv => concat!(
                    "env vars and the current directory are process-global; mutating\n",
                    "them without the shared lock stomps on sibling tests in the same\n",
                    "binary. Take the lock first:\n",
                    "    let _guard = gwt_core::test_support::env_lock().lock().unwrap();\n",
                    "or scope the change with gwt_core::test_support::ScopedEnvVar.\n",
                    "If the mutation genuinely cannot race, declare it:\n",
                    "    // test-hygiene: allow-unlocked-env <reason>"
                ),
                Rule::NativeWatcherDeadline => concat!(
                    "Native filesystem delivery has no deterministic wall-clock bound.\n",
                    "Test the subscription/delivery contract without waiting for the OS.\n",
                    "A retained real-OS integration probe requires an explicit reason:\n",
                    "    // test-hygiene: allow-native-watcher-deadline <reason>"
                ),
                Rule::ProductionProbeDeadline => concat!(
                    "ResolvedContainerRuntime::resolve starts a real process under a\n",
                    "production wall-clock deadline, even with no Duration in the test.\n",
                    "Inject the probe result for pure contracts. If executing the real\n",
                    "wrapper is essential to this test, declare why:\n",
                    "    // test-hygiene: allow-production-probe-deadline <reason>"
                ),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SourceKind {
        /// Everything in the file is test code (`crates/*/tests/**`).
        WholeFileIsTest,
        /// Only `#[cfg(test)]` items are test code (`crates/*/src/**`).
        CfgTestBlocksOnly,
    }

    #[derive(Debug, Clone)]
    pub struct Finding {
        pub rule: Rule,
        pub file: String,
        pub line: usize,
        pub context: String,
        pub snippet: String,
    }

    impl Finding {
        pub fn baseline_key(&self) -> String {
            format!("{}|{}|{}", self.rule.id(), self.file, self.context)
        }
    }

    #[derive(Debug, Clone)]
    pub struct Source {
        pub relative_path: String,
        pub kind: SourceKind,
    }

    /// Files a `#[cfg(test)] mod <name>;` declaration pulls in.
    ///
    /// Such a file holds nothing but test code, yet the attribute that says so
    /// sits in the *declaring* module. Classifying it by its own contents made
    /// every line of it unscannable — 15 modules in this workspace, including
    /// `app_runtime/tests.rs`, which kept a `Duration::from_millis(25)` with a
    /// green gate and no baseline row.
    fn externally_declared_test_modules(files: &[PathBuf]) -> std::collections::BTreeSet<PathBuf> {
        let declaration = Regex::new(r"^\s*#\[cfg\(test\)\]\s*$").expect("cfg(test) line");
        let module = Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
            .expect("mod declaration");

        let mut declared = std::collections::BTreeSet::new();
        for file in files {
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            let lines: Vec<&str> = text.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if !declaration.is_match(line) {
                    continue;
                }
                // The attribute may be followed by other attributes before the
                // declaration itself; skip them rather than give up.
                let Some(next) = lines[index + 1..]
                    .iter()
                    .find(|candidate| !candidate.trim().starts_with("#["))
                else {
                    continue;
                };
                let Some(captures) = module.captures(next) else {
                    continue;
                };
                let name = &captures[1];
                // `foo/mod.rs` and `lib.rs` declare siblings; `foo.rs` declares
                // children of its own directory.
                let base = match file.file_name().and_then(|n| n.to_str()) {
                    Some("mod.rs") | Some("lib.rs") | Some("main.rs") => {
                        file.parent().map(Path::to_path_buf)
                    }
                    _ => file.parent().map(|dir| dir.join(file.file_stem().unwrap())),
                };
                let Some(base) = base else { continue };
                for candidate in [
                    base.join(format!("{name}.rs")),
                    base.join(name).join("mod.rs"),
                ] {
                    if candidate.is_file() {
                        declared.insert(candidate);
                    }
                }
            }
        }
        declared
    }

    /// Every Rust file in the workspace that can hold test code.
    pub fn target_sources(root: &Path) -> Vec<Source> {
        let mut sources = Vec::new();
        let crates = root.join("crates");
        let mut crate_dirs: Vec<PathBuf> = std::fs::read_dir(&crates)
            .unwrap_or_else(|error| panic!("read {}: {error}", crates.display()))
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        crate_dirs.sort();

        for crate_dir in crate_dirs {
            for (sub, kind) in [
                ("tests", SourceKind::WholeFileIsTest),
                ("src", SourceKind::CfgTestBlocksOnly),
            ] {
                let dir = crate_dir.join(sub);
                if !dir.is_dir() {
                    continue;
                }
                let mut files = Vec::new();
                collect_rust_files(&dir, &mut files);
                files.sort();
                let declared_test_modules = match kind {
                    SourceKind::WholeFileIsTest => Default::default(),
                    SourceKind::CfgTestBlocksOnly => externally_declared_test_modules(&files),
                };
                for file in files {
                    let relative = file
                        .strip_prefix(root)
                        .expect("scanned file lives under the workspace root")
                        .to_string_lossy()
                        .replace('\\', "/");
                    let kind = if declared_test_modules.contains(&file) {
                        SourceKind::WholeFileIsTest
                    } else {
                        kind
                    };
                    sources.push(Source {
                        relative_path: relative,
                        kind,
                    });
                }
            }
        }

        sources
    }

    fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                collect_rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    pub fn scan_source(file: &str, text: &str, kind: SourceKind) -> Vec<Finding> {
        let raw: Vec<&str> = text.lines().collect();
        let code = strip_literals_and_comments(&raw);
        let depths = brace_depths(&code);
        let functions = function_ranges(&code, &depths);
        let scannable = scannable_lines(&code, &depths, kind);

        let mut findings = Vec::new();
        for (index, line) in code.iter().enumerate() {
            if !scannable[index] {
                continue;
            }
            let enclosing = enclosing_function(&functions, index);
            for rule in [
                Rule::ShortWallClock,
                Rule::UnlockedProcessEnv,
                Rule::NativeWatcherDeadline,
                Rule::ProductionProbeDeadline,
            ] {
                if !line_violates(rule, line) {
                    continue;
                }
                if declared_allowed(rule, &raw, index) {
                    continue;
                }
                if rule == Rule::NativeWatcherDeadline
                    && !enclosing.is_some_and(|range| {
                        let body = code[range.start..=range.end].join(" ");
                        body.contains("recv_batch") && body.contains("timeout")
                    })
                {
                    continue;
                }
                if rule == Rule::UnlockedProcessEnv
                    && enclosing.is_some_and(|range| holds_env_lock(&code, range))
                {
                    continue;
                }
                findings.push(Finding {
                    rule,
                    file: file.to_string(),
                    line: index + 1,
                    context: enclosing
                        .map(|range| range.name.clone())
                        .unwrap_or_else(|| MODULE_CONTEXT.to_string()),
                    snippet: raw[index].trim().to_string(),
                });
            }
        }

        findings
    }

    /// Context recorded for a violation that sits outside any function.
    const MODULE_CONTEXT: &str = "<module>";

    /// Identifiers that prove the shared process-state lock is held.
    const ENV_LOCK_MARKERS: [&str; 4] =
        ["env_test_lock", "env_lock", "ScopedEnvVar", "ScopedGwtHome"];

    fn line_violates(rule: Rule, code: &str) -> bool {
        match rule {
            Rule::ShortWallClock => {
                if sub_millisecond_regex().is_match(code) {
                    return true;
                }
                short_millis_regex().captures_iter(code).any(|capture| {
                    capture[1]
                        .replace('_', "")
                        .parse::<u64>()
                        .is_ok_and(|value| value < SHORT_DURATION_THRESHOLD_MS)
                })
            }
            Rule::UnlockedProcessEnv => env_mutation_regex().is_match(code),
            Rule::NativeWatcherDeadline => native_watcher_regex().is_match(code),
            Rule::ProductionProbeDeadline => production_probe_regex().is_match(code),
        }
    }

    /// An exception counts only when the marker carries a reason, on the
    /// offending line itself or on the line directly above it.
    fn declared_allowed(rule: Rule, raw: &[&str], index: usize) -> bool {
        let marker = rule.allow_marker();
        let mut candidates = vec![raw[index]];
        if index > 0 {
            candidates.push(raw[index - 1]);
        }
        candidates.iter().any(|line| match line.find(marker) {
            Some(at) => !line[at + marker.len()..].trim().is_empty(),
            None => false,
        })
    }

    fn holds_env_lock(code: &[String], range: &FunctionRange) -> bool {
        code[range.start..=range.end]
            .iter()
            .any(|line| ENV_LOCK_MARKERS.iter().any(|marker| line.contains(marker)))
    }

    #[derive(Debug, Clone)]
    struct FunctionRange {
        name: String,
        start: usize,
        end: usize,
    }

    /// Innermost function containing `index`, if any.
    fn enclosing_function(functions: &[FunctionRange], index: usize) -> Option<&FunctionRange> {
        functions
            .iter()
            .rfind(|range| range.start <= index && index <= range.end)
    }

    fn function_ranges(code: &[String], depths: &[BraceDepth]) -> Vec<FunctionRange> {
        let mut ranges = Vec::new();
        for (index, line) in code.iter().enumerate() {
            let Some(capture) = fn_declaration_regex().captures(line) else {
                continue;
            };
            let end = block_end(code, depths, index);
            ranges.push(FunctionRange {
                name: capture[1].to_string(),
                start: index,
                end,
            });
        }
        ranges
    }

    fn scannable_lines(code: &[String], depths: &[BraceDepth], kind: SourceKind) -> Vec<bool> {
        match kind {
            SourceKind::WholeFileIsTest => vec![true; code.len()],
            SourceKind::CfgTestBlocksOnly => {
                let mut scannable = vec![false; code.len()];
                for (index, line) in code.iter().enumerate() {
                    // `#[cfg(not(test))]` is the opposite of a test block, so
                    // it must not pull production code into the scan.
                    if !cfg_test_regex().is_match(line) || line.contains("not(") {
                        continue;
                    }
                    let end = block_end(code, depths, index);
                    for flag in scannable.iter_mut().take(end + 1).skip(index) {
                        *flag = true;
                    }
                }
                scannable
            }
        }
    }

    /// Last line of the item that starts at `start`: the line where the brace
    /// depth returns to where it was, or the statement's own `;` when the item
    /// has no block at all.
    fn block_end(code: &[String], depths: &[BraceDepth], start: usize) -> usize {
        let outer = depths[start].before;
        let mut opened = false;
        for index in start..code.len() {
            if code[index].contains('{') {
                opened = true;
            }
            if opened && depths[index].after <= outer {
                return index;
            }
            if !opened && code[index].contains(';') {
                return index;
            }
        }
        code.len().saturating_sub(1)
    }

    #[derive(Debug, Clone, Copy)]
    struct BraceDepth {
        before: i32,
        after: i32,
    }

    fn brace_depths(code: &[String]) -> Vec<BraceDepth> {
        let mut depths = Vec::with_capacity(code.len());
        let mut depth = 0_i32;
        for line in code {
            let before = depth;
            for ch in line.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            depths.push(BraceDepth {
                before,
                after: depth,
            });
        }
        depths
    }

    /// Blanks out comments, string literals and char literals so that a
    /// `Duration::from_millis(1)` written inside a fixture string or a doc
    /// comment is not mistaken for test code. Braces and quotes inside those
    /// spans would otherwise also desynchronise the depth tracking.
    fn strip_literals_and_comments(raw: &[&str]) -> Vec<String> {
        let mut block_comment_depth = 0_usize;
        let mut raw_string_hashes: Option<usize> = None;
        raw.iter()
            .map(|line| strip_line(line, &mut block_comment_depth, &mut raw_string_hashes))
            .collect()
    }

    fn strip_line(
        line: &str,
        block_comment_depth: &mut usize,
        raw_string_hashes: &mut Option<usize>,
    ) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::with_capacity(line.len());
        let mut index = 0;

        while index < chars.len() {
            if let Some(hashes) = *raw_string_hashes {
                if chars[index] == '"' && trailing_hashes(&chars, index + 1) >= hashes {
                    *raw_string_hashes = None;
                    index += 1 + hashes;
                } else {
                    index += 1;
                }
                continue;
            }

            if *block_comment_depth > 0 {
                if starts_with(&chars, index, "*/") {
                    *block_comment_depth -= 1;
                    index += 2;
                } else if starts_with(&chars, index, "/*") {
                    *block_comment_depth += 1;
                    index += 2;
                } else {
                    index += 1;
                }
                continue;
            }

            if starts_with(&chars, index, "//") {
                break;
            }
            if starts_with(&chars, index, "/*") {
                *block_comment_depth += 1;
                index += 2;
                continue;
            }

            if chars[index] == 'r' && !is_ident_char(chars.get(index.wrapping_sub(1)).copied()) {
                let hashes = trailing_hashes(&chars, index + 1);
                if chars.get(index + 1 + hashes) == Some(&'"') {
                    *raw_string_hashes = Some(hashes);
                    index += 1 + hashes + 1;
                    continue;
                }
            }

            if chars[index] == '"' {
                index += 1;
                while index < chars.len() {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == '"' {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
                continue;
            }

            if chars[index] == '\'' {
                let mut end = index + 1;
                if chars.get(end) == Some(&'\\') {
                    end += 2;
                } else {
                    end += 1;
                }
                if chars.get(end) == Some(&'\'') {
                    index = end + 1;
                    continue;
                }
            }

            out.push(chars[index]);
            index += 1;
        }

        out
    }

    fn trailing_hashes(chars: &[char], from: usize) -> usize {
        chars[from.min(chars.len())..]
            .iter()
            .take_while(|ch| **ch == '#')
            .count()
    }

    fn starts_with(chars: &[char], index: usize, pattern: &str) -> bool {
        pattern
            .chars()
            .enumerate()
            .all(|(offset, expected)| chars.get(index + offset) == Some(&expected))
    }

    fn is_ident_char(ch: Option<char>) -> bool {
        ch.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    }

    fn short_millis_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| {
            Regex::new(r"\bDuration::from_millis\(\s*([0-9_]+)\s*\)").expect("valid regex")
        })
    }

    fn native_watcher_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"\bstart_watcher\s*\(").expect("valid regex"))
    }

    fn production_probe_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| {
            Regex::new(r"\bResolvedContainerRuntime\s*::\s*resolve\s*\(").expect("valid regex")
        })
    }

    fn sub_millisecond_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"\bDuration::from_(?:nanos|micros)\(").expect("valid regex"))
    }

    fn env_mutation_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| {
            Regex::new(r"\benv::(?:set_var|remove_var|set_current_dir)\s*\(").expect("valid regex")
        })
    }

    fn fn_declaration_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)").expect("valid regex"))
    }

    fn cfg_test_regex() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"#\[cfg\((?:[^\]]*[(,\s])?test[),\s]").expect("valid regex"))
    }
}
