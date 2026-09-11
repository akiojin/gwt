//! Verification-plan derivation from changed surfaces (SPEC-3248 full
//! T-130 core).
//!
//! `verify.plan` with `params.derive:true` classifies the worktree's
//! changed files — branch changes against the `origin/develop` merge-base
//! when available, plus uncommitted changes and untracked files — into
//! surfaces and derives the verification matrix from them, instead of
//! trusting the agent to hand-pick commands:
//!
//! - **rust** (`crates/<name>/…`, workspace manifests): CI's fmt and clippy
//!   gates verbatim, plus the CI Rust test gate scoped to each changed
//!   crate (the whole workspace gate when a workspace manifest changed).
//! - **skills / guidance** (`.claude/skills/`, `.codex/skills/`): the
//!   `gwt-skills` test suite (managed-asset parity lives there).
//! - **frontend** (`crates/gwt/web/` and js/ts/css/html): the embedded web
//!   contract tests, which live in the `gwt` crate.
//! - **docs** (markdown outside the skill trees): `bunx markdownlint-cli2`
//!   over the changed files (AGENTS markdown policy).
//! - **anything else** (scripts, CI config, …): the conservative default —
//!   the CI lint gates + the `gwt` crate suite.
//!
//! # Package narrowing, never target narrowing
//!
//! The commands are CI's own gates ([`CI_RUST_TEST_GATE`], [`CI_FMT_GATE`],
//! [`CI_CLIPPY_GATE`]) narrowed to the changed packages, and nothing else.
//! Narrowing by *target* instead is what made this derivation unsound: the
//! `gwt` crate's matrix used to be `cargo test -p gwt --lib`, which never
//! ran the ~1300 unit tests living in its binary targets (`app_runtime` and
//! the rest of `main.rs`'s module tree, plus `gwtd`) nor its integration
//! tests. A change confined to those targets derived a matrix that could
//! not fail, so `verify.run` reported GREEN on work that CI then rejected
//! (#3640). Tests in this module pin the derived commands against the
//! workflow files so CI cannot drift away from them unnoticed.
//!
//! # The one exception: the `gwt` crate's binary targets on Windows
//!
//! A Windows host cannot run those, and no amount of waiting changes that.
//! Four `app_runtime` tests take `env_test_lock` and never release it
//! (#4014), so the run wedges with the test binary's CPU flat, a pile of
//! orphaned `cmd /d /s /c "exit /b 0"` children, and — because the wedged
//! process keeps holding it — the host-wide verification lease. One such run
//! starved every other worktree on the machine for hours, and since
//! `execution.reopen` and the Ready PR gate both consume a passing derived
//! record, finished work could not ship while it sat there (#4182).
//! Serializing with `--test-threads=1` only moves the wedge later; linking
//! those targets also fails outright with `os error 5`, because Windows
//! cannot replace the very `gwtd.exe` that is running the verification
//! (#3808, #4172).
//!
//! So on Windows the `gwt` package — and the workspace gate, which contains
//! it — narrows to `--lib`, and every derived `cargo test` is serialized.
//! This follows CI rather than departing from it: `test-windows-default-parallel`
//! runs exactly [`CI_WINDOWS_RUST_TEST_GATE`] and documents the same
//! deadlock as its reason for excluding the target. Every other package
//! keeps CI's full gate, because only these targets have ever been observed
//! to wedge — narrowing further would buy nothing and cost real coverage.
//! Windows verification is weaker than Linux's as a result, and CI stays the
//! gate that decides; a local run that cannot finish decides nothing at all.
//!
//! The derived plan is a DEFAULT, not a cage: explicit `verify.plan`
//! commands stay supported, and the recorded plan carries `derived: true`
//! so downstream review can tell the two apart. Acceptance-scenario-driven
//! derivation and plan floor policies remain follow-ups (T-130 full).

use std::{collections::BTreeSet, path::Path};

use gwt_core::process::hidden_command;

/// The Rust test gate CI runs on every pull request (`.github/workflows/
/// test.yml`, job `test`). It is the single source of truth for the derived
/// matrix: derivation narrows it by **package**, never by **target**.
const CI_RUST_TEST_GATE: &str = "cargo test --workspace --all-features";
/// CI's formatting gate (`.github/workflows/lint.yml`, job `lint`).
const CI_FMT_GATE: &str = "cargo fmt --all -- --check";
/// CI's clippy gate (`.github/workflows/lint.yml`, job `lint`).
const CI_CLIPPY_GATE: &str = "cargo clippy --workspace --all-targets --all-features -- -D warnings";

/// The broad Rust test gate CI runs on Windows (`.github/workflows/
/// test.yml`, job `test-windows-default-parallel`): the same gate restricted
/// to library targets, because the `gwt` crate's binary targets deadlock
/// there. Derivation applies the identical restriction — see the module
/// header.
const CI_WINDOWS_RUST_TEST_GATE: &str = "cargo test --workspace --lib --all-features";

/// The only package whose binary targets are known to wedge a Windows host,
/// and the only one derivation narrows there (#4014, #4182).
const WINDOWS_DEADLOCKING_PACKAGE: &str = "gwt";

/// Which host the derived matrix has to be runnable on.
///
/// Derivation is host-sensitive because CI's own Rust matrix is (#4182).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationHost {
    /// Windows, where the `gwt` crate's binary targets deadlock.
    Windows,
    /// Every other host, where CI's full gate runs as written.
    Other,
}

impl VerificationHost {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// The CI Rust gate scoped to one package. Package selection is the only
/// narrowing derivation is allowed to apply — a target filter such as
/// `--lib` drops whole test families (the `gwt` crate's binary targets carry
/// ~1300 unit tests) while still reporting a GREEN verification run (#3640).
///
/// Windows narrows one package's targets on top of that, because it cannot
/// run them at all — see the module header.
fn package_test_command_for(package: &str, host: VerificationHost) -> String {
    rust_test_command_for(Some(package), host)
}

/// CI's unnarrowed Rust gate for `host`, used when a change cannot be
/// attributed to any single package.
fn workspace_test_command_for(host: VerificationHost) -> String {
    rust_test_command_for(None, host)
}

/// The derived Rust test command for one package, or for the whole
/// workspace when `package` is `None`.
fn rust_test_command_for(package: Option<&str>, host: VerificationHost) -> String {
    // A workspace-wide gate builds the deadlocking package too, so both
    // spellings take CI's Windows gate.
    let takes_windows_gate = host == VerificationHost::Windows
        && package.is_none_or(|package| package == WINDOWS_DEADLOCKING_PACKAGE);
    let gate = if takes_windows_gate {
        CI_WINDOWS_RUST_TEST_GATE
    } else {
        CI_RUST_TEST_GATE
    };
    let scope = match package {
        Some(package) => format!("-p {package}"),
        None => "--workspace".to_string(),
    };
    let mut command = gate.replace("--workspace", &scope);
    if host == VerificationHost::Windows {
        // Serialization is not the fix for that deadlock — it only defers
        // it — but the targeted Windows CI steps run serialized because
        // several own external resources (a real PTY, a console subsystem),
        // and a derived matrix cannot tell which of those it is about to run.
        command.push_str(" -- --test-threads=1");
    }
    command
}

/// A derived verification plan: the matrix plus the surface classification
/// that produced it (echoed to the agent for transparency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedPlan {
    pub commands: Vec<String>,
    pub surfaces: Vec<String>,
    pub trivial_reason: Option<TrivialReason>,
}

/// Stable reasons why a derived plan has no runnable verification target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrivialReason {
    LedgerOnly,
    DeletionOnly,
    IntegrationBranch,
    MergeBaseUnavailable,
}

impl TrivialReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LedgerOnly => "ledger_only",
            Self::DeletionOnly => "deletion_only",
            Self::IntegrationBranch => "integration_branch",
            Self::MergeBaseUnavailable => "merge_base_unavailable",
        }
    }
}

impl DerivedPlan {
    fn trivial(reason: TrivialReason) -> Self {
        Self {
            commands: Vec::new(),
            surfaces: vec![format!("trivial({})", reason.as_str())],
            trivial_reason: Some(reason),
        }
    }
}

fn git_lines(worktree: &Path, args: &[&str]) -> Vec<String> {
    hidden_command("git")
        .arg("-C")
        .arg(worktree)
        // Non-ASCII paths must come back verbatim, not quote-escaped —
        // escaped spellings would defeat every classifier and exclusion.
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve the integration base the committed span is diffed against.
/// Fail-closed: without a resolvable base the committed branch work would
/// silently vanish from the matrix, so derivation refuses instead.
pub(crate) fn integration_merge_base(worktree: &Path) -> Option<String> {
    for base_ref in ["origin/develop", "origin/main", "origin/HEAD"] {
        if let Some(base) = git_lines(worktree, &["merge-base", base_ref, "HEAD"])
            .into_iter()
            .next()
        {
            return Some(base);
        }
    }
    None
}

/// Collect the changed paths: the committed span against the integration
/// merge-base, uncommitted changes against HEAD, and untracked files. gwt
/// bookkeeping under `.gwt/` and `tasks/` never counts as a surface.
fn changed_paths(worktree: &Path) -> Result<Vec<String>, TrivialReason> {
    // On the integration branch itself the committed span is unattributable
    // (merge-base == HEAD hides already-pushed work) — refuse rather than
    // derive a silently weak matrix.
    let head_branch = git_lines(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
        .into_iter()
        .next()
        .unwrap_or_default();
    if matches!(head_branch.as_str(), "develop" | "main" | "master") {
        return Err(TrivialReason::IntegrationBranch);
    }
    let base = integration_merge_base(worktree).ok_or(TrivialReason::MergeBaseUnavailable)?;
    let mut paths: BTreeSet<String> = BTreeSet::new();
    paths.extend(git_lines(worktree, &["diff", "--name-only", &base, "HEAD"]));
    paths.extend(git_lines(worktree, &["diff", "--name-only", "HEAD"]));
    paths.extend(git_lines(
        worktree,
        &["ls-files", "--others", "--exclude-standard"],
    ));
    Ok(paths
        .into_iter()
        .filter(|path| !path.starts_with(".gwt/") && !path.starts_with("tasks/"))
        .collect())
}

fn crate_of(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("crates/")?;
    let (name, _tail) = rest.split_once('/')?;
    Some(name)
}

fn is_rust_path(path: &str) -> bool {
    path.ends_with(".rs") || path.ends_with("Cargo.toml") || path.ends_with("Cargo.lock")
}

fn is_skills_path(path: &str) -> bool {
    path.starts_with(".claude/skills/") || path.starts_with(".codex/skills/")
}

fn is_frontend_path(path: &str) -> bool {
    path.starts_with("crates/gwt/web/")
        || [".js", ".mjs", ".ts", ".css", ".html"]
            .iter()
            .any(|ext| path.ends_with(ext))
}

fn is_docs_path(path: &str) -> bool {
    path.ends_with(".md") && !is_skills_path(path)
}

/// Derive the verification matrix from the worktree's changed surfaces.
/// A no-target change set is represented by a reason-bearing trivial plan.
/// Non-git directories remain invalid because they cannot provide a stable
/// worktree fingerprint.
pub fn derive(worktree: &Path) -> Result<DerivedPlan, String> {
    derive_for_host(worktree, VerificationHost::current())
}

/// [`derive()`] against an explicit host, so both branches of the
/// host-sensitive matrix stay reachable from tests on any machine (#4182).
fn derive_for_host(worktree: &Path, host: VerificationHost) -> Result<DerivedPlan, String> {
    if git_lines(worktree, &["rev-parse", "--git-dir"]).is_empty() {
        return Err("verify.plan derive requires a git worktree".to_string());
    }
    let paths = match changed_paths(worktree) {
        Ok(paths) => paths,
        Err(reason) => return Ok(DerivedPlan::trivial(reason)),
    };
    if paths.is_empty() {
        return Ok(DerivedPlan::trivial(TrivialReason::LedgerOnly));
    }

    let mut rust_crates: BTreeSet<String> = BTreeSet::new();
    let mut workspace_rust = false;
    let mut skills = false;
    let mut frontend = false;
    let mut docs_files: Vec<String> = Vec::new();
    let mut other = false;

    for path in &paths {
        if is_skills_path(path) {
            skills = true;
        } else if is_docs_path(path) {
            docs_files.push(path.clone());
        } else if is_frontend_path(path) {
            frontend = true;
        } else if is_rust_path(path) {
            match crate_of(path) {
                Some(name) => {
                    rust_crates.insert(name.to_string());
                }
                None => workspace_rust = true,
            }
        } else {
            other = true;
        }
    }
    // Rust changes inside gwt-skills are also the skills surface.
    if rust_crates.contains("gwt-skills") {
        skills = true;
    }

    let mut surfaces: Vec<String> = Vec::new();
    let mut commands: Vec<String> = Vec::new();
    let push_unique = |commands: &mut Vec<String>, command: String| {
        if !commands.contains(&command) {
            commands.push(command);
        }
    };

    let code_changed = !rust_crates.is_empty() || workspace_rust || skills || frontend || other;
    if code_changed {
        push_unique(&mut commands, CI_FMT_GATE.to_string());
        push_unique(&mut commands, CI_CLIPPY_GATE.to_string());
    }
    // Which packages the changed surfaces put under test. Narrowing stops
    // here: every package runs CI's whole gate, never a target subset.
    let mut test_packages: BTreeSet<&str> = BTreeSet::new();
    if !rust_crates.is_empty() || workspace_rust {
        surfaces.push(format!(
            "rust({})",
            if rust_crates.is_empty() {
                "workspace".to_string()
            } else {
                rust_crates.iter().cloned().collect::<Vec<_>>().join(",")
            }
        ));
        test_packages.extend(rust_crates.iter().map(String::as_str));
    }
    if skills {
        surfaces.push("skills".to_string());
        test_packages.insert("gwt-skills");
    }
    if frontend {
        surfaces.push("frontend".to_string());
        test_packages.insert("gwt");
    }
    if other {
        surfaces.push("other".to_string());
        test_packages.insert("gwt");
    }
    if workspace_rust {
        // A workspace manifest change cannot be attributed to any single
        // package, so it takes CI's gate unnarrowed — which subsumes every
        // per-package command the surfaces above would have added.
        push_unique(&mut commands, workspace_test_command_for(host));
    } else {
        for package in test_packages {
            push_unique(&mut commands, package_test_command_for(package, host));
        }
    }
    // Only lint files that still exist — a deleted path would make
    // markdownlint-cli2 exit 0 on zero matches (a vacuous PASS), and paths
    // are quoted so spaces survive the runner's tokenizer.
    docs_files.retain(|path| worktree.join(path).exists());
    if !docs_files.is_empty() {
        surfaces.push(format!("docs({})", docs_files.len()));
        let quoted: Vec<String> = docs_files
            .iter()
            .map(|path| format!("\"{path}\""))
            .collect();
        push_unique(
            &mut commands,
            format!("bunx markdownlint-cli2 {}", quoted.join(" ")),
        );
    }
    if commands.is_empty() {
        return Ok(DerivedPlan::trivial(TrivialReason::DeletionOnly));
    }

    Ok(DerivedPlan {
        commands,
        surfaces,
        trivial_reason: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(worktree: &Path, rel: &str, contents: &str) {
        let path = worktree.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn git(worktree: &Path, args: &[&str]) {
        let status = hidden_command("git")
            .arg("-C")
            .arg(worktree)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// Fixture: repo with an integration base recorded as
    /// `origin/develop`, work continuing on a feature branch (the shape
    /// gwt launches produce).
    fn fixture(worktree: &Path) {
        crate::cli::trusted_store::init_git_repo_with_origin(worktree);
        git(
            worktree,
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(worktree, &["checkout", "-q", "-b", "work/fixture"]);
    }

    // T-130: rust + skills + docs surfaces derive the combined matrix, in
    // stable order, without duplicates.
    #[test]
    fn derives_matrix_from_mixed_surfaces() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "crates/gwt-core/src/lib.rs", "pub fn x() {}");
        write(dir.path(), "crates/gwt/src/main.rs", "fn main() {}");
        write(dir.path(), ".claude/skills/gwt-verify/SKILL.md", "# skill");
        write(dir.path(), "README.md", "# readme");
        // Bookkeeping never counts.
        write(dir.path(), ".gwt/work/events.jsonl", "{}");
        write(dir.path(), "tasks/todo.md", "- [ ] x");

        let plan = derive_for_host(dir.path(), VerificationHost::Other).unwrap();
        assert_eq!(
            plan.commands,
            vec![
                CI_FMT_GATE.to_string(),
                CI_CLIPPY_GATE.to_string(),
                "cargo test -p gwt --all-features".to_string(),
                "cargo test -p gwt-core --all-features".to_string(),
                "cargo test -p gwt-skills --all-features".to_string(),
                r#"bunx markdownlint-cli2 "README.md""#.to_string(),
            ],
            "{:?}",
            plan.surfaces
        );
        assert!(plan.surfaces.iter().any(|s| s.starts_with("rust(")));
        assert!(plan.surfaces.contains(&"skills".to_string()));
    }

    // Docs-only changes derive only the markdown lint — no vacuous cargo
    // matrix, but never an empty plan.
    #[test]
    fn docs_only_derives_markdownlint_only() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "README.md", "# readme");
        write(dir.path(), "docs/guide.md", "# guide");

        let plan = derive(dir.path()).unwrap();
        assert_eq!(
            plan.commands,
            vec![r#"bunx markdownlint-cli2 "README.md" "docs/guide.md""#.to_string()]
        );
    }

    // DE-1: committed branch work counts through the merge-base leg, while
    // an unresolvable base or integration branch produces an explicit
    // no-target verification plan instead of a recovery dead end.
    #[test]
    fn committed_branch_changes_join_the_matrix() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "crates/gwt-core/src/lib.rs", "pub fn x() {}");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "feat: core change"]);
        // Only a doc is dirty now — the committed rust must still derive.
        write(dir.path(), "README.md", "# readme");

        let plan = derive_for_host(dir.path(), VerificationHost::Other).unwrap();
        assert!(
            plan.commands
                .contains(&"cargo test -p gwt-core --all-features".to_string()),
            "{:?}",
            plan.commands
        );
        assert!(plan.commands.contains(&CI_CLIPPY_GATE.to_string()));
    }

    #[test]
    fn unresolvable_base_and_integration_branch_are_trivial() {
        // No origin/develop|main|HEAD refs at all.
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        git(dir.path(), &["checkout", "-q", "-b", "work/fixture"]);
        write(dir.path(), "README.md", "# readme");
        let plan = derive(dir.path()).unwrap();
        assert!(plan.commands.is_empty());
        assert_eq!(
            plan.trivial_reason,
            Some(TrivialReason::MergeBaseUnavailable)
        );

        // Sitting on the integration branch itself.
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(dir.path(), &["checkout", "-q", "-B", "develop"]);
        write(dir.path(), "README.md", "# readme");
        let plan = derive(dir.path()).unwrap();
        assert!(plan.commands.is_empty());
        assert_eq!(plan.trivial_reason, Some(TrivialReason::IntegrationBranch));
    }

    // Deletions-only change sets produce an explicit no-target plan rather
    // than a vacuous markdownlint invocation.
    #[test]
    fn deleted_docs_only_is_trivial() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "notes.md", "# notes");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "docs: notes"]);
        std::fs::remove_file(dir.path().join("notes.md")).unwrap();

        let plan = derive(dir.path()).unwrap();
        assert!(plan.commands.is_empty());
        assert_eq!(plan.trivial_reason, Some(TrivialReason::DeletionOnly));
    }

    // No non-bookkeeping changes are represented as an explicit ledger-only
    // plan. Non-git directories still refuse.
    #[test]
    fn empty_is_trivial_and_non_git_refuses() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let plan = derive(dir.path()).unwrap();
        assert!(plan.commands.is_empty());
        assert_eq!(plan.trivial_reason, Some(TrivialReason::LedgerOnly));

        let plain = tempfile::tempdir().unwrap();
        let err = derive(plain.path()).unwrap_err();
        assert!(err.contains("git worktree"), "{err}");
    }

    /// Every `run:` script the named job executes, in step order.
    fn workflow_job_runs(workflow: &str, job: &str) -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root above crates/gwt")
            .join(".github/workflows")
            .join(workflow);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).expect("workflow is valid YAML");
        doc["jobs"][job]["steps"]
            .as_sequence()
            .unwrap_or_else(|| panic!("{workflow} job `{job}` has steps"))
            .iter()
            .filter_map(|step| Some(step.get("run")?.as_str()?.to_string()))
            .collect()
    }

    /// Every cargo test invocation the job runs, one per script line.
    fn workflow_cargo_tests(workflow: &str, job: &str) -> Vec<String> {
        workflow_job_runs(workflow, job)
            .iter()
            .flat_map(|script| script.lines().collect::<Vec<_>>())
            .map(str::trim)
            .filter(|line| line.contains("cargo test"))
            .map(str::to_string)
            .collect()
    }

    /// The derived matrix for every surface, for invariant sweeps.
    fn derive_for(files: &[&str]) -> DerivedPlan {
        derive_on(VerificationHost::Other, files)
    }

    /// The same sweep pinned to one host, so both branches of the
    /// host-sensitive matrix are exercised wherever the suite runs (#4182).
    fn derive_on(host: VerificationHost, files: &[&str]) -> DerivedPlan {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        for file in files {
            write(dir.path(), file, "x\n");
        }
        derive_for_host(dir.path(), host).unwrap()
    }

    /// Every `cargo test` command in a derived matrix.
    fn cargo_tests(plan: &DerivedPlan) -> Vec<String> {
        plan.commands
            .iter()
            .filter(|command| command.starts_with("cargo test"))
            .cloned()
            .collect()
    }

    // #4182 AC-1 / AC-6: the derived matrix must not ask a Windows host to
    // run the `gwt` crate's binary targets. Four `app_runtime` tests take
    // `env_test_lock` and never release it there (#4014), so the run wedges
    // with the test binary's CPU flat and a pile of orphaned
    // `cmd /d /s /c "exit /b 0"` children — while still holding the
    // host-wide verification lease. CI already refuses to run that target on
    // Windows; only the locally derived matrix still demanded it.
    #[test]
    fn windows_derivation_drops_the_deadlocking_gwt_bin_target() {
        let plan = derive_on(
            VerificationHost::Windows,
            &["crates/gwt/src/app_runtime.rs"],
        );
        assert!(
            plan.commands.contains(
                &"cargo test -p gwt --lib --all-features -- --test-threads=1".to_string()
            ),
            "{:?}",
            plan.commands
        );
        assert!(
            !plan
                .commands
                .contains(&"cargo test -p gwt --all-features".to_string()),
            "the full gate deadlocks on Windows: {:?}",
            plan.commands
        );
    }

    // #4182 AC-1: whichever packages a Windows change puts under test, every
    // derived `cargo test` is serialized. The targeted Windows CI steps pin
    // `--test-threads=1` per fixture because they own external resources (a
    // real PTY, a console subsystem); a locally derived matrix cannot tell
    // which of those it is about to run, so it serializes all of them.
    #[test]
    fn windows_derived_cargo_tests_are_serialized() {
        for files in [
            vec!["crates/gwt-core/src/lib.rs"],
            vec!["Cargo.toml"],
            vec!["scripts/release.sh"],
            vec!["crates/gwt/web/styles/tokens.css"],
        ] {
            let plan = derive_on(VerificationHost::Windows, &files);
            let tests = cargo_tests(&plan);
            assert!(!tests.is_empty(), "{files:?}: {:?}", plan.commands);
            for command in tests {
                assert!(
                    command.ends_with(" -- --test-threads=1"),
                    "{files:?}: `{command}` is not serialized"
                );
            }
        }
    }

    // #4182 AC-8 / AC-9: the run that holds the verification lease is itself
    // a `gwtd` process, and Windows cannot replace a file that is open. A
    // derived matrix that relinks `target/debug/gwtd.exe` therefore fails
    // with `os error 5` every time (#3808, #4172). Restricting the Windows
    // matrix to library targets makes that structurally impossible, because
    // `--lib` builds no binary targets at all.
    #[test]
    fn windows_derivation_never_relinks_the_running_gwtd() {
        for files in [
            vec!["crates/gwt/src/main.rs"],
            vec!["Cargo.toml"],
            vec!["scripts/release.sh"],
        ] {
            for command in cargo_tests(&derive_on(VerificationHost::Windows, &files)) {
                for target in ["--bins", "--bin ", "--all-targets", "--tests", "--test "] {
                    assert!(
                        !command.contains(target),
                        "{files:?}: `{command}` builds binary targets ({target}) and would \
                         relink the running gwtd"
                    );
                }
                assert!(
                    command.contains(" --lib "),
                    "{files:?}: `{command}` is not restricted to library targets"
                );
            }
        }
    }

    // #4182 AC-6: the Windows matrix is CI's own Windows gate narrowed by
    // package, exactly as the default matrix is CI's Linux gate narrowed by
    // package. This fails the moment CI's Windows job changes shape and the
    // derivation is not updated with it.
    #[test]
    fn windows_derived_rust_matrix_tracks_the_ci_windows_gate() {
        // CI narrows its own Windows gate to `-p gwt`, and builds it on a
        // separate budget before the timed loop, so the job runs the gate
        // twice: once with `--no-run`, once for real.
        let gwt_gate = CI_WINDOWS_RUST_TEST_GATE.replace("--workspace", "-p gwt");
        assert_eq!(
            workflow_cargo_tests("test.yml", "test-windows-default-parallel"),
            vec![format!("{gwt_gate} --no-run"), gwt_gate.clone()],
            "CI's Windows Rust gate changed — update verify.plan derivation with it (#4182)"
        );
        assert_eq!(
            package_test_command_for("gwt", VerificationHost::Windows),
            format!("{gwt_gate} -- --test-threads=1")
        );
        assert!(
            derive_on(VerificationHost::Windows, &["Cargo.toml"])
                .commands
                .contains(&format!("{CI_WINDOWS_RUST_TEST_GATE} -- --test-threads=1")),
            "workspace manifest change must derive the full Windows gate"
        );
    }

    // #4182: the Windows narrowing is as small as the evidence. Only the
    // `gwt` crate's binary targets have ever wedged a Windows host, so only
    // they are dropped — every other package keeps CI's full gate there,
    // integration tests included. Narrowing further would trade real
    // coverage for nothing, which is the #3640 failure mode in reverse.
    #[test]
    fn windows_narrowing_is_confined_to_the_deadlocking_package() {
        assert_eq!(
            package_test_command_for("gwt-core", VerificationHost::Windows),
            "cargo test -p gwt-core --all-features -- --test-threads=1"
        );
        assert_eq!(
            package_test_command_for("gwt-skills", VerificationHost::Windows),
            "cargo test -p gwt-skills --all-features -- --test-threads=1"
        );
        let plan = derive_on(VerificationHost::Windows, &["crates/gwt-core/src/lib.rs"]);
        assert!(
            plan.commands
                .contains(&"cargo test -p gwt-core --all-features -- --test-threads=1".to_string()),
            "{:?}",
            plan.commands
        );
    }

    // #4182 AC-5: the Windows split changes nothing anywhere else. Linux and
    // macOS keep running CI's full `--all-features` gate, target filters and
    // all, because the deadlock it exists to avoid is Windows-only.
    #[test]
    fn non_windows_derivation_is_untouched_by_the_windows_split() {
        for files in [
            vec!["crates/gwt/src/app_runtime.rs"],
            vec!["crates/gwt-core/src/lib.rs"],
            vec!["Cargo.toml"],
            vec!["scripts/release.sh"],
        ] {
            let plan = derive_on(VerificationHost::Other, &files);
            let tests = cargo_tests(&plan);
            assert!(!tests.is_empty(), "{files:?}: {:?}", plan.commands);
            for command in tests {
                assert!(
                    !command.contains("--lib") && !command.contains("--test-threads"),
                    "{files:?}: `{command}` leaked the Windows narrowing"
                );
            }
        }
        assert_eq!(
            package_test_command_for("gwt-core", VerificationHost::Other),
            "cargo test -p gwt-core --all-features"
        );
    }

    // #3640 AC-1: the `gwt` crate's binary targets carry ~1300 unit tests
    // (`app_runtime` and the rest of main.rs's module tree, plus gwtd). The
    // old `--lib`-only command never ran a single one of them, so a change
    // living entirely in a bin-only module derived a matrix that could not
    // fail — local GREEN, CI RED.
    #[test]
    fn gwt_bin_only_modules_are_covered_by_the_derived_matrix() {
        let plan = derive_for(&["crates/gwt/src/app_runtime.rs"]);
        assert!(
            plan.commands
                .contains(&"cargo test -p gwt --all-features".to_string()),
            "{:?}",
            plan.commands
        );
    }

    // #3640 AC-2 / AC-4: derivation may narrow the CI gate by PACKAGE, never
    // by TARGET. Any target filter silently drops a family of tests that CI
    // still runs, which is exactly how `--bin gwt` went unverified.
    #[test]
    fn derived_cargo_tests_narrow_by_package_only() {
        let surfaces: [(&str, &[&str]); 6] = [
            ("gwt crate", &["crates/gwt/src/cli/verify_derivation.rs"]),
            ("other crate", &["crates/gwt-core/src/lib.rs"]),
            ("workspace manifest", &["Cargo.toml"]),
            ("frontend", &["crates/gwt/web/styles/tokens.css"]),
            ("skills", &[".claude/skills/gwt-verify/SKILL.md"]),
            ("unknown", &["scripts/release.sh"]),
        ];
        for (label, files) in surfaces {
            let plan = derive_for(files);
            let tests: Vec<&String> = plan
                .commands
                .iter()
                .filter(|command| command.starts_with("cargo test"))
                .collect();
            assert!(!tests.is_empty(), "{label}: {:?}", plan.commands);
            for command in tests {
                for filter in [
                    "--lib",
                    "--bins",
                    "--bin ",
                    "--tests",
                    "--test ",
                    "--examples",
                    "--benches",
                    "--doc",
                ] {
                    assert!(
                        !command.contains(filter),
                        "{label}: `{command}` narrows the CI gate by target ({filter})"
                    );
                }
                assert!(
                    command.contains("--all-features"),
                    "{label}: `{command}` does not match the CI gate's feature selection"
                );
            }
        }
    }

    // #3640 AC-3 / AC-4: the derived matrix is defined as the CI gate
    // narrowed by package, so this test fails the moment CI's Rust job
    // changes shape and the derivation is not updated with it.
    #[test]
    fn derived_rust_matrix_tracks_the_ci_rust_gate() {
        assert_eq!(
            workflow_cargo_tests("test.yml", "test"),
            vec![
                CI_RUST_TEST_GATE.to_string(),
                // The xvfb `--ignored` real-binary family stays CI-owned:
                // it needs a display server, so it is deliberately outside
                // the locally derived matrix.
                "xvfb-run -a cargo test -p gwt --all-features --test stable_server_port -- --ignored --test-threads=1"
                    .to_string(),
            ],
            "CI's Rust gate changed — update verify.plan derivation with it (#3640)"
        );
        assert_eq!(
            package_test_command_for("gwt-core", VerificationHost::Other),
            "cargo test -p gwt-core --all-features"
        );
        // A workspace-manifest change cannot be attributed to one package,
        // so it derives the CI gate verbatim.
        assert!(
            derive_for(&["Cargo.toml"])
                .commands
                .contains(&CI_RUST_TEST_GATE.to_string()),
            "workspace manifest change must derive the full CI gate"
        );
    }

    // #3640 AC-3: `cargo fmt`/`cargo clippy` were narrowed to the workspace
    // default member (`crates/gwt`), so a rustfmt or clippy violation in any
    // other crate passed local verification and failed CI.
    #[test]
    fn derived_lint_commands_track_the_ci_lint_gate() {
        let runs = workflow_job_runs("lint.yml", "lint");
        for gate in [CI_CLIPPY_GATE, CI_FMT_GATE] {
            assert!(
                runs.iter().any(|run| run.trim() == gate),
                "CI's lint gate no longer runs `{gate}` — update verify.plan derivation with it (#3640)"
            );
        }
        let plan = derive_for(&["crates/gwt-core/src/lib.rs"]);
        assert!(plan.commands.contains(&CI_FMT_GATE.to_string()), "{plan:?}");
        assert!(
            plan.commands.contains(&CI_CLIPPY_GATE.to_string()),
            "{plan:?}"
        );
    }

    // Unknown surfaces (scripts, CI config) fall back to the conservative
    // default matrix.
    #[test]
    fn unknown_surface_gets_conservative_default() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "scripts/release.sh", "#!/bin/sh\n");

        let plan = derive_for_host(dir.path(), VerificationHost::Other).unwrap();
        assert!(plan.commands.contains(&CI_CLIPPY_GATE.to_string()));
        assert!(plan
            .commands
            .contains(&"cargo test -p gwt --all-features".to_string()));
        assert!(plan.surfaces.contains(&"other".to_string()));
    }
}
