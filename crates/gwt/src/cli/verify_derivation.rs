//! Verification-plan derivation from changed surfaces (SPEC-3248 full
//! T-130 core).
//!
//! `verify.plan` with `params.derive:true` classifies the worktree's
//! changed files — branch changes against the `origin/develop` merge-base
//! when available, plus uncommitted changes and untracked files — into
//! surfaces and derives the verification matrix from them, instead of
//! trusting the agent to hand-pick commands:
//!
//! - **rust** (`crates/<name>/…`, workspace manifests): CI's fmt, clippy, and
//!   rustdoc gates verbatim, plus the CI Rust test gate scoped to each changed
//!   crate (the whole workspace gate when a workspace manifest changed).
//! - **skills / guidance** (`.claude/skills/`, `.codex/skills/`): the
//!   `gwt-skills` test suite (managed-asset parity lives there).
//! - **frontend** (`crates/gwt/web/` and js/ts/css/html): the embedded web
//!   contract tests, which live in the `gwt` crate.
//! - **node scripts** (`scripts/*.js`, `*.mjs`, `*.cjs`): existing sibling
//!   `*.test.*` files through `node --test`.
//! - **docs** (markdown outside the skill trees): `bunx markdownlint-cli2`
//!   over the changed files (AGENTS markdown policy).
//! - **anything else**: the conservative Rust matrix plus an explicit
//!   `unsupported(path)` diagnostic. Automatic registration refuses unknown
//!   surfaces; an explicit plan supplies their project-specific commands.
//!
//! # Package narrowing, never target narrowing
//!
//! The commands are CI's own gates ([`CI_RUST_TEST_GATE`], [`CI_FMT_GATE`],
//! [`CI_CLIPPY_GATE`], [`CI_RUSTDOC_GATE`]) narrowed to the changed packages,
//! and nothing else.
//! Narrowing by *target* instead is what made this derivation unsound: the
//! `gwt` crate's matrix used to be `cargo test -p gwt --lib`, which never
//! ran the ~1300 unit tests living in its binary targets (`app_runtime` and
//! the rest of `main.rs`'s module tree, plus `gwtd`) nor its integration
//! tests. A change confined to those targets derived a matrix that could
//! not fail, so `verify.run` reported GREEN on work that CI then rejected
//! (#3640). Tests in this module pin the derived commands against the
//! workflow files so CI cannot drift away from them unnoticed.
//!
//! # Windows: library and binary unit tests without integration targets
//!
//! This narrowing used to cover `--bin gwt` as well. A Windows run wedged
//! with the test binary's CPU flat and a pile of orphaned
//! `cmd /d /s /c "exit /b 0"` children, and — because the wedged process
//! kept holding it — the host-wide verification lease. One such run starved
//! every other worktree on the machine for hours, and since
//! `execution.reopen` and the Ready PR gate both consume a passing derived
//! record, finished work could not ship while it sat there (#4182).
//!
//! #4014 found the cause and removed it: the window close finalizer joined
//! the PTY reader thread while the pane still held the pseudoconsole open,
//! and Windows ConPTY does not signal EOF on the output pipe until the
//! pseudoconsole is closed, so that join could never return. It ran under
//! `env_test_lock`, which is why one hung teardown looked like four wedged
//! tests — the other three were only queued behind the lock. `--bin gwt` is
//! back in the nightly Windows gate as a result.
//!
//! Integration tests cause Cargo to build the regular binaries, including
//! the live `target/debug/gwtd.exe`, which Windows cannot replace (#3808,
//! #4172). Binary unit tests build separate test harnesses instead. Selecting
//! `--lib --bins` covers both `gwt` and `gwtd` unit tests without rebuilding
//! the running controller (#4968). The portable fallback stays serialized.
//! Projects declaring a `gwt-verify` nextest profile opt into process-isolated
//! tests and JUnit evidence on Windows (#4822), with resource scheduling owned
//! by that profile. The same target coverage is preserved, including separate
//! rustdoc commands when the original gate included doctests.
//!
//! Every other package keeps CI's full gate, because its integration tests
//! do not build the live controller.
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
/// CI's rustdoc gate (`.github/workflows/lint.yml`, job `lint`). The step's
/// `env: RUSTDOCFLAGS` is written as a leading assignment because `cargo doc`
/// has no `-- -D warnings`; `verify.run` applies such a prefix as process
/// environment (#3698).
const CI_RUSTDOC_GATE: &str =
    r#"RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items"#;

/// The Rust unit-test gate derivation uses on Windows. Explicit library and
/// binary targets avoid integration tests rebuilding the live `gwtd.exe`.
/// The nightly `test-windows-default-parallel` job selects `--bin gwt`
/// instead of all binary unit-test harnesses. The test
/// `windows_derived_rust_matrix_tracks_the_ci_windows_gate` pins that exact
/// relationship so neither side can drift alone.
const CI_WINDOWS_RUST_TEST_GATE: &str = "cargo test --workspace --lib --bins --all-features";

/// The package whose integration tests rebuild the running controller.
const WINDOWS_CONTROLLER_PACKAGE: &str = "gwt";

/// Which host the derived matrix has to be runnable on.
///
/// Derivation is host-sensitive because CI's own Rust matrix is (#4182).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationHost {
    /// Windows, where `gwt` integration tests rebuild the live controller.
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
    // A workspace-wide gate builds the controller package too, so both
    // spellings take CI's Windows gate.
    let takes_windows_gate = host == VerificationHost::Windows
        && package.is_none_or(|package| package == WINDOWS_CONTROLLER_PACKAGE);
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
    pub(crate) fn unsupported_reason(&self) -> Option<String> {
        let unsupported: Vec<&str> = self
            .surfaces
            .iter()
            .filter(|surface| surface.starts_with("unsupported("))
            .map(String::as_str)
            .collect();
        (!unsupported.is_empty()).then(|| {
            format!(
                "verify.plan derive has unsupported changed surfaces [{}]; register an explicit verify.plan with params.commands covering these paths before canonical verification",
                unsupported.join(", ")
            )
        })
    }

    fn trivial(reason: TrivialReason) -> Self {
        Self {
            commands: Vec::new(),
            surfaces: vec![format!("trivial({})", reason.as_str())],
            trivial_reason: Some(reason),
        }
    }
}

fn git_lines(worktree: &Path, args: &[&str]) -> Vec<String> {
    checked_git_lines(worktree, args).unwrap_or_default()
}

fn checked_git_lines(worktree: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let output = hidden_command("git")
        .arg("-C")
        .arg(worktree)
        // Non-ASCII paths must come back verbatim, not quote-escaped —
        // escaped spellings would defeat every classifier and exclusion.
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .output()
        .map_err(|error| format!("git {} failed: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("git {} returned unreadable paths: {error}", args.join(" ")))?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
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
    changed_source_paths_since(worktree, &base).map_err(|_| TrivialReason::MergeBaseUnavailable)
}

/// The source paths `worktree` holds that `base` does not: the committed span,
/// uncommitted changes against HEAD, and untracked files, with `.gwt/` and
/// `tasks/` bookkeeping excluded. Shared with the delivered-owner classifier so
/// "nothing to deliver" means exactly what the verification matrix means by it.
pub(crate) fn changed_source_paths_since(
    worktree: &Path,
    base: &str,
) -> Result<Vec<String>, String> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    paths.extend(checked_git_lines(
        worktree,
        &["diff", "--no-renames", "--name-only", base, "HEAD"],
    )?);
    paths.extend(checked_git_lines(
        worktree,
        &["diff", "--no-renames", "--name-only", "HEAD"],
    )?);
    paths.extend(checked_git_lines(
        worktree,
        &["ls-files", "--others", "--exclude-standard"],
    )?);
    Ok(paths
        .into_iter()
        .filter(|path| {
            super::delivery_paths::classify_path(path.as_bytes())
                == super::delivery_paths::DeliveryPath::Product
        })
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

fn is_node_script_path(path: &str) -> bool {
    path.starts_with("scripts/")
        && [".js", ".mjs", ".cjs"]
            .iter()
            .any(|ext| path.ends_with(ext))
}

fn node_test_paths(worktree: &Path, path: &str) -> Vec<String> {
    if is_frontend_test_path(path) {
        return worktree
            .join(path)
            .is_file()
            .then(|| path.to_string())
            .into_iter()
            .collect();
    }
    let Some((stem, _)) = path.rsplit_once('.') else {
        return Vec::new();
    };
    ["mjs", "js", "cjs"]
        .iter()
        .map(|ext| format!("{stem}.test.{ext}"))
        .filter(|test| worktree.join(test).is_file())
        .collect()
}

/// A frontend path that exercises the UI rather than rendering it.
///
/// Issue #4510: these still belong to the frontend *matrix* — changing a
/// Playwright spec is exactly the reason to run the Playwright suite — but
/// they are not a UI *surface*, because there is no rendered change for a
/// human to look at. Conflating the two made a PR whose entire diff was one
/// `*.spec.ts` demand a visual confirmation nobody could give (PR #4374).
fn is_frontend_test_path(path: &str) -> bool {
    path.starts_with("crates/gwt/playwright/tests/")
        || path.contains("/__tests__/")
        || [
            ".spec.ts",
            ".spec.js",
            ".test.ts",
            ".test.js",
            ".test.mjs",
            ".test.cjs",
        ]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

/// Whether a changed path renders UI a human could be asked to look at.
///
/// Issue #4637: outside `crates/gwt/web/` a script extension alone proves
/// nothing is rendered (`scripts/*.mjs` is tooling), so only markup and
/// stylesheets keep the doubt-means-UI default there.
fn is_ui_surface_path(path: &str) -> bool {
    is_frontend_path(path)
        && !is_frontend_test_path(path)
        && (path.starts_with("crates/gwt/web/")
            || [".css", ".html"].iter().any(|ext| path.ends_with(ext)))
}

/// Inspect frontend changes even when plan derivation is trivial on an
/// integration branch. An unknown base or unreadable diff cannot prove that
/// a Ready handoff has no UI surface.
pub fn has_frontend_changes(worktree: &Path) -> Result<bool, String> {
    let base = integration_merge_base(worktree)
        .ok_or_else(|| "frontend classification requires a readable git merge-base".to_string())?;
    Ok(changed_source_paths_since(worktree, &base)?
        .iter()
        .any(|path| is_ui_surface_path(path)))
}

fn is_docs_path(path: &str) -> bool {
    path.ends_with(".md") && !is_skills_path(path)
}

/// Derive the verification matrix from the worktree's changed surfaces.
/// A no-target change set is represented by a reason-bearing trivial plan.
/// Non-git directories remain invalid because they cannot provide a stable
/// worktree fingerprint.
#[cfg(test)]
pub fn derive(worktree: &Path) -> Result<DerivedPlan, String> {
    derive_excluding(worktree, &[])
}

pub(crate) fn derive_excluding(
    worktree: &Path,
    generated_outputs: &[String],
) -> Result<DerivedPlan, String> {
    derive_for_host_excluding(worktree, VerificationHost::current(), generated_outputs)
}

/// Read live protected contexts before taking the trusted store writer lease.
pub fn read_pre_pr_required_contexts(worktree: &Path) -> Result<Vec<String>, String> {
    if worktree.join("crates/gwt/Cargo.toml").is_file() {
        let read = |args: &[&str]| -> Result<String, String> {
            let output = hidden_command("gh")
                .current_dir(worktree)
                .args(args)
                .output()
                .map_err(|err| {
                    format!("pre-pr required CI read failed: {err}; use verify.plan mode full")
                })?;
            if !output.status.success() {
                return Err(format!(
                    "pre-pr required CI read failed: {}; use verify.plan mode full",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        };
        let repo = read(&[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ])?;
        let endpoint = format!("repos/{repo}/branches/develop/protection");
        serde_json::from_str::<Vec<String>>(&read(&[
            "api",
            &endpoint,
            "--jq",
            ".required_status_checks.contexts",
        ])?)
        .map_err(|err| format!("pre-pr required contexts unreadable: {err}"))
    } else {
        Ok(Vec::new())
    }
}

/// Explicit pre-PR policy for the gwt repository. Other projects keep their
/// full matrix. Required contexts come from the live read at registration.
pub fn derive_pre_pr(
    worktree: &Path,
    required: &[String],
    acceptance: &[String],
    local: &[String],
    generated_outputs: &[String],
) -> Result<DerivedPlan, String> {
    derive_pre_pr_for_host_excluding(
        worktree,
        VerificationHost::current(),
        required,
        acceptance,
        local,
        generated_outputs,
    )
}

#[cfg(test)]
fn derive_pre_pr_for_host(
    worktree: &Path,
    host: VerificationHost,
    required: &[String],
    acceptance: &[String],
    local: &[String],
) -> Result<DerivedPlan, String> {
    derive_pre_pr_for_host_excluding(worktree, host, required, acceptance, local, &[])
}

fn derive_pre_pr_for_host_excluding(
    worktree: &Path,
    host: VerificationHost,
    required: &[String],
    acceptance: &[String],
    local: &[String],
    generated_outputs: &[String],
) -> Result<DerivedPlan, String> {
    let mut plan = derive_for_host_excluding(worktree, host, generated_outputs)?;
    if plan.trivial_reason.is_some() {
        for command in acceptance.iter().chain(local) {
            if command.trim().is_empty() {
                return Err("pre-pr commands must not be empty".into());
            }
            if !plan.commands.contains(command) {
                plan.commands.push(command.clone());
            }
        }
        if !plan.commands.is_empty() {
            plan.trivial_reason = None;
            plan.surfaces = vec!["explicit-checks".into()];
        }
        return Ok(plan);
    }
    if !worktree.join("crates/gwt/Cargo.toml").is_file() {
        for command in acceptance.iter().chain(local) {
            if !plan.commands.contains(command) {
                plan.commands.push(command.clone());
            }
        }
        return Ok(plan);
    }
    if acceptance.is_empty() || acceptance.iter().any(|c| c.trim().is_empty()) {
        return Err("pre-pr requires non-empty acceptance_commands fixing this Issue's AC; use mode full otherwise".into());
    }
    validate_pre_pr_ci(worktree, required)?;
    let mut packages = BTreeSet::new();
    let mut workspace = false;
    for path in changed_paths(worktree).map_err(|reason| reason.as_str().to_string())? {
        if generated_outputs.contains(&path) {
            continue;
        }
        if is_rust_path(&path) {
            if let Some(package) = crate_of(&path) {
                packages.insert(package.to_string());
            } else {
                workspace = true;
            }
        } else if is_skills_path(&path) {
            packages.insert("gwt-skills".to_string());
        } else if !is_docs_path(&path) {
            packages.insert("gwt".to_string());
        }
    }
    // Required Rust/coverage contexts do not cover the derived Node runner.
    let node_commands: Vec<_> = plan
        .commands
        .drain(..)
        .filter(|command| command.starts_with("node --test "))
        .collect();
    if workspace || !packages.is_empty() {
        plan.commands.push(CI_FMT_GATE.to_string());
        let scope = if workspace {
            "--workspace".into()
        } else {
            packages
                .iter()
                .map(|p| format!("-p {p}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        plan.commands
            .push(CI_CLIPPY_GATE.replace("--workspace", &scope));
    }
    plan.commands.extend(node_commands);
    for command in acceptance.iter().chain(local) {
        if command.trim().is_empty() {
            return Err("pre-pr commands must not be empty".into());
        }
        if !plan.commands.contains(command) {
            plan.commands.push(command.clone());
        }
    }
    plan.surfaces.push("ci-delegated(rust-tests->Test (Rust);workspace-clippy,rustdoc,markdown->Clippy & Rustfmt;coverage90/80->coverage / Rust Coverage)".into());
    Ok(plan)
}

/// The correspondence is intentionally small and repository-specific. A
/// missing required context, job, trigger, or failure guard refuses delegation.
fn validate_pre_pr_ci(worktree: &Path, required: &[String]) -> Result<(), String> {
    use serde_yaml::Value;
    let require = |condition: bool, missing: &str| -> Result<(), String> {
        if condition {
            Ok(())
        } else {
            Err(format!(
                "pre-pr CI contract missing {missing}; repair CI or use verify.plan mode full"
            ))
        }
    };
    for context in [
        "Test (Rust)",
        "Clippy & Rustfmt",
        "coverage / Rust Coverage",
    ] {
        require(
            required.iter().any(|name| name == context),
            &format!("required context {context}"),
        )?;
    }
    let load = |name: &str| -> Result<Value, String> {
        let text = std::fs::read_to_string(worktree.join(".github/workflows").join(name))
            .map_err(|err| format!("pre-pr CI contract {name}: {err}"))?;
        serde_yaml::from_str(&text).map_err(|err| format!("pre-pr CI contract {name}: {err}"))
    };
    let test = load("test.yml")?;
    let lint = load("lint.yml")?;
    let coverage = load("coverage.yml")?;
    for (name, doc) in [("test.yml", &test), ("lint.yml", &lint)] {
        require(
            doc["on"].get("pull_request").is_some() && doc["on"].get("merge_group").is_some(),
            &format!("{name} PR/merge_group triggers"),
        )?;
        require(
            doc["on"]["pull_request"].get("paths").is_none()
                && doc["on"]["pull_request"].get("paths-ignore").is_none(),
            &format!("unfiltered {name} PR trigger"),
        )?;
    }
    let contains = |job: &Value, command: &str| {
        job["steps"].as_sequence().into_iter().flatten().any(|s| {
            s.get("if").is_none()
                && s.get("continue-on-error").is_none()
                && s["run"]
                    .as_str()
                    .is_some_and(|run| run.lines().any(|line| line.trim() == command))
        })
    };
    let depends = |job: &Value, dependency: &str| {
        job["needs"].as_str() == Some(dependency)
            || job["needs"]
                .as_sequence()
                .is_some_and(|needs| needs.iter().any(|n| n.as_str() == Some(dependency)))
    };
    let rust_gate = &test["jobs"]["test-rust-required"];
    require(
        rust_gate["name"].as_str() == Some("Test (Rust)")
            && depends(rust_gate, "test")
            && rust_gate["if"].as_str() == Some("${{ !cancelled() }}")
            && contains(rust_gate, "test \"$RUST_RESULT\" = success")
            && rust_gate["steps"][0]["env"]["RUST_RESULT"].as_str()
                == Some("${{ needs.test.result }}"),
        "Test (Rust) workspace failure propagation",
    )?;
    require(
        contains(
            &test["jobs"]["test"],
            "cargo nextest run --workspace --all-features --test-threads=1",
        ) && contains(
            &test["jobs"]["test"],
            "cargo test --workspace --all-features --doc",
        ),
        "required workspace tests/doctests",
    )?;
    let lint_gate = &lint["jobs"]["lint"];
    require(
        lint_gate["name"].as_str() == Some("Clippy & Rustfmt")
            && lint_gate.get("needs").is_none()
            && lint_gate.get("if").is_none(),
        "independent Clippy & Rustfmt job",
    )?;
    require(
        contains(lint_gate, CI_CLIPPY_GATE)
            && contains(
                lint_gate,
                "cargo doc --workspace --no-deps --document-private-items",
            ),
        "required clippy/rustdoc",
    )?;
    require(
        lint_gate["steps"].as_sequence().is_some_and(|steps| {
            steps.iter().any(|s| {
                s["run"].as_str()
                    == Some("cargo doc --workspace --no-deps --document-private-items")
                    && s["env"]["RUSTDOCFLAGS"].as_str() == Some("-D warnings")
            })
        }),
        "required rustdoc warnings",
    )?;
    require(
        lint_gate["steps"].as_sequence().is_some_and(|steps| {
            steps.iter().any(|s| {
                s["uses"]
                    .as_str()
                    .is_some_and(|u| u.starts_with("nosborn/github-action-markdown-cli@"))
                    && s["with"]["files"].as_str() == Some(".")
            })
        }),
        "required markdown lint",
    )?;
    require(
        lint["jobs"]["coverage"]["uses"].as_str() == Some("./.github/workflows/coverage.yml")
            && lint["jobs"]["coverage"].get("if").is_none()
            && lint["jobs"]["coverage"].get("continue-on-error").is_none()
            && coverage["on"].get("workflow_call").is_some(),
        "required reusable coverage job",
    )?;
    let coverage_job = &coverage["jobs"]["rust-coverage"];
    require(
        lint["jobs"]["coverage"]["name"]
            .as_str()
            .unwrap_or("coverage")
            == "coverage"
            && coverage_job["name"].as_str() == Some("Rust Coverage"),
        "coverage / Rust Coverage check name",
    )?;
    for command in ["node scripts/coverage-summary.mjs --output-path target/coverage-summary.json -- --workspace --all-features", "node scripts/check-coverage-threshold.mjs target/coverage-summary.json 90 --scope \"crates/(gwt-core|gwt)/\"", "node scripts/check-coverage-threshold.mjs target/coverage-summary.json 80 --scope-exclude \"crates/(gwt-core|gwt)/\""] {
        require(contains(coverage_job, command), &format!("required coverage command {command}"))?;
    }
    for job in [&test["jobs"]["test"], rust_gate, lint_gate, coverage_job] {
        require(
            job.get("continue-on-error").is_none()
                && job["steps"].as_sequence().is_some_and(|steps| {
                    steps
                        .iter()
                        .filter(|s| {
                            s.get("run").is_some()
                                || s["uses"]
                                    .as_str()
                                    .is_some_and(|u| u.starts_with("nosborn/"))
                        })
                        .all(|s| {
                            s.get("continue-on-error").is_none()
                                && s.get("if").is_none_or(|_| {
                                    s["name"].as_str().is_some_and(|n| {
                                        n == "Check for processes the suite failed to reap"
                                            || n == "Generate lcov"
                                    })
                                })
                        })
                }),
            "unconditional non-advisory delegated steps",
        )?;
    }
    require(
        coverage_job.get("if").is_none()
            // A source-sync failure must not skip the workspace tests. This
            // status-only guard overrides implicit success() without filtering sources.
            && test["jobs"]["test"].get("if").is_none_or(|condition| {
                condition.as_str() == Some("${{ !cancelled() }}")
            }),
        "unconditional delegated jobs",
    )?;
    Ok(())
}

/// [`derive()`] against an explicit host, so both branches of the
/// host-sensitive matrix stay reachable from tests on any machine (#4182).
#[cfg(test)]
fn derive_for_host(worktree: &Path, host: VerificationHost) -> Result<DerivedPlan, String> {
    derive_for_host_excluding(worktree, host, &[])
}

fn derive_for_host_excluding(
    worktree: &Path,
    host: VerificationHost,
    generated_outputs: &[String],
) -> Result<DerivedPlan, String> {
    if git_lines(worktree, &["rev-parse", "--git-dir"]).is_empty() {
        return Err("verify.plan derive requires a git worktree".to_string());
    }
    let paths = match changed_paths(worktree) {
        Ok(paths) => paths
            .into_iter()
            .filter(|path| !generated_outputs.contains(path))
            .collect::<Vec<_>>(),
        Err(reason) => return Ok(DerivedPlan::trivial(reason)),
    };
    if paths.is_empty() {
        return Ok(DerivedPlan::trivial(TrivialReason::LedgerOnly));
    }

    // Only mark the config covered when a derived runner actually reads it.
    let nextest_profile = host == VerificationHost::Windows
        && std::fs::read_to_string(worktree.join(".config/nextest.toml"))
            .ok()
            .and_then(|config| toml::from_str::<toml::Value>(&config).ok())
            .is_some_and(|config| {
                config
                    .get("profile")
                    .and_then(|p| p.get("gwt-verify"))
                    .is_some()
            });

    let mut rust_crates: BTreeSet<String> = BTreeSet::new();
    let mut workspace_rust = false;
    let mut skills = false;
    let mut frontend = false;
    let mut ui_surface = false;
    let mut docs_files: Vec<String> = Vec::new();
    let mut other = false;
    let mut node_tests = BTreeSet::new();
    let mut unsupported = Vec::new();

    for path in &paths {
        if is_skills_path(path) {
            skills = true;
        } else if is_docs_path(path) {
            docs_files.push(path.clone());
        } else if is_node_script_path(path) {
            let tests = node_test_paths(worktree, path);
            if tests.is_empty() {
                other = true;
                unsupported.push(format!("unsupported({path})"));
            } else {
                node_tests.extend(tests);
            }
        } else if is_frontend_path(path) {
            frontend = true;
            ui_surface |= is_ui_surface_path(path);
        } else if is_rust_path(path) {
            match crate_of(path) {
                Some(name) => {
                    rust_crates.insert(name.to_string());
                }
                None => workspace_rust = true,
            }
        } else {
            other = true;
            if path != ".config/nextest.toml" || !nextest_profile {
                unsupported.push(format!("unsupported({path})"));
            }
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
        push_unique(&mut commands, CI_RUSTDOC_GATE.to_string());
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
        // Issue #4510: the matrix is the same either way, but the label is the
        // only thing the Ready handoff reads to decide whether a human has
        // anything to look at. A test-only frontend change declares itself as
        // such so the visual gate is not raised over a `*.spec.ts`.
        surfaces.push(if ui_surface {
            "frontend".to_string()
        } else {
            "frontend-tests".to_string()
        });
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
    // Projects opt in through their own nextest profile. Do not require an
    // additional runner in unrelated projects or change non-Windows gates.
    if nextest_profile {
        let mut nextest_commands = Vec::new();
        for command in commands {
            if let Some(selection) = command.strip_prefix("cargo test ") {
                let selection = selection.trim_end_matches(" -- --test-threads=1");
                nextest_commands.push(format!(
                    "cargo nextest run {selection} --profile gwt-verify --retries 0"
                ));
                // nextest does not run rustdoc. A library-only cargo test
                // never did either; preserve that existing target boundary.
                if !selection.split_whitespace().any(|arg| arg == "--lib") {
                    nextest_commands.push(format!("cargo test {selection} --doc"));
                }
            } else {
                nextest_commands.push(command);
            }
        }
        commands = nextest_commands;
    }

    if !node_tests.is_empty() {
        surfaces.push("node-tests".to_string());
        push_unique(
            &mut commands,
            format!(
                "node --test {}",
                node_tests
                    .iter()
                    .map(|path| format!("\"{path}\""))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
    }
    surfaces.extend(unsupported);

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

    fn pre_pr_fixture(worktree: &Path) {
        fixture(worktree);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for path in [
            "crates/gwt/Cargo.toml",
            ".github/workflows/test.yml",
            ".github/workflows/lint.yml",
            ".github/workflows/coverage.yml",
        ] {
            write(
                worktree,
                path,
                &std::fs::read_to_string(root.join(path)).unwrap(),
            );
        }
        write(worktree, "crates/gwt/src/change.rs", "// changed\n");
    }

    #[test]
    fn pre_pr_refuses_coverage_without_its_own_required_context() {
        let dir = tempfile::tempdir().unwrap();
        pre_pr_fixture(dir.path());
        let result = validate_pre_pr_ci(
            dir.path(),
            &["Test (Rust)".into(), "Clippy & Rustfmt".into()],
        );
        assert!(
            result
                .unwrap_err()
                .contains("required context coverage / Rust Coverage"),
            "optional coverage must not permit delegating the threshold checks"
        );
    }

    #[test]
    fn pre_pr_delegates_only_to_required_contexts_and_keeps_ac_and_local_checks() {
        let dir = tempfile::tempdir().unwrap();
        pre_pr_fixture(dir.path());
        let required = vec![
            "Test (Rust)".to_string(),
            "Clippy & Rustfmt".to_string(),
            "coverage / Rust Coverage".to_string(),
        ];
        let ac = "cargo test -p gwt --test ci_pre_pr_contract_test".to_string();
        let local = "bash scripts/local-only-check.sh".to_string();
        let plan = derive_pre_pr_for_host(
            dir.path(),
            VerificationHost::Other,
            &required,
            std::slice::from_ref(&ac),
            std::slice::from_ref(&local),
        )
        .unwrap();
        assert!(plan.commands.contains(&CI_FMT_GATE.to_string()));
        assert!(plan.commands.contains(
            &"cargo clippy -p gwt --all-targets --all-features -- -D warnings".to_string()
        ));
        assert!(plan.commands.contains(&ac));
        assert!(plan.commands.contains(&local));
        assert!(!plan.commands.iter().any(|c| c.starts_with("RUSTDOCFLAGS=")
            || c == "cargo test -p gwt --all-features"
            || c == CI_CLIPPY_GATE));
        assert!(plan.surfaces.iter().any(|s| s.contains("Test (Rust)")
            && s.contains("Clippy & Rustfmt")
            && s.contains("coverage90/80->coverage / Rust Coverage")));
        assert!(derive_pre_pr_for_host(
            dir.path(),
            VerificationHost::Other,
            &[],
            std::slice::from_ref(&ac),
            &[]
        )
        .unwrap_err()
        .contains("required"));
        // A running optional job is not an admissible replacement.
        assert!(derive_pre_pr_for_host(
            dir.path(),
            VerificationHost::Other,
            &["Test (Rust workspace)".into(), "Clippy & Rustfmt".into()],
            std::slice::from_ref(&ac),
            &[]
        )
        .is_err());
        assert!(
            derive_pre_pr_for_host(dir.path(), VerificationHost::Other, &required, &[], &[])
                .unwrap_err()
                .contains("acceptance_commands")
        );
        assert!(derive(dir.path())
            .unwrap()
            .commands
            .contains(&CI_CLIPPY_GATE.to_string()));
        std::fs::remove_file(dir.path().join(".github/workflows/test.yml")).unwrap();
        assert!(
            derive_pre_pr_for_host(dir.path(), VerificationHost::Other, &required, &[ac], &[])
                .unwrap_err()
                .contains("test.yml")
        );
    }

    #[test]
    fn pre_pr_workspace_job_accepts_only_the_cancellation_status_guard() {
        let dir = tempfile::tempdir().unwrap();
        pre_pr_fixture(dir.path());
        let required = vec![
            "Test (Rust)".into(),
            "Clippy & Rustfmt".into(),
            "coverage / Rust Coverage".into(),
        ];
        let original =
            std::fs::read_to_string(dir.path().join(".github/workflows/test.yml")).unwrap();
        let guarded = "  test:\n    name: Test (Rust workspace)\n    if: ${{ !cancelled() }}\n";
        for (condition, accepted) in [
            ("", true),
            ("    if: ${{ !cancelled() }}\n", true),
            ("    if: github.event_name == 'pull_request'\n", false),
            (
                "    if: ${{ !cancelled() && needs.source-sync.outputs.base_only == 'false' }}\n",
                false,
            ),
        ] {
            let replacement = format!("  test:\n    name: Test (Rust workspace)\n{condition}");
            assert!(original.contains(guarded));
            write(
                dir.path(),
                ".github/workflows/test.yml",
                &original.replace(guarded, &replacement),
            );
            let result = validate_pre_pr_ci(dir.path(), &required);
            assert_eq!(result.is_ok(), accepted, "{condition:?}: {result:?}");
        }
    }

    #[test]
    fn pre_pr_refuses_removed_ci_jobs_or_serial_lint() {
        let dir = tempfile::tempdir().unwrap();
        pre_pr_fixture(dir.path());
        let required = vec![
            "Test (Rust)".into(),
            "Clippy & Rustfmt".into(),
            "coverage / Rust Coverage".into(),
        ];
        let original =
            std::fs::read_to_string(dir.path().join(".github/workflows/lint.yml")).unwrap();
        for mutation in [
            original.replace("  coverage:\n", "  optional-coverage:\n"),
            original.replace(
                "    name: Clippy & Rustfmt\n",
                "    name: Clippy & Rustfmt\n    needs: coverage\n",
            ),
            original.replace("Clippy & Rustfmt", "Optional lint"),
        ] {
            write(dir.path(), ".github/workflows/lint.yml", &mutation);
            assert!(derive_pre_pr_for_host(
                dir.path(),
                VerificationHost::Other,
                &required,
                &["cargo test -p gwt --test ci_pre_pr_contract_test".into()],
                &[]
            )
            .is_err());
        }
        write(dir.path(), ".github/workflows/lint.yml", &original);
        let coverage =
            std::fs::read_to_string(dir.path().join(".github/workflows/coverage.yml")).unwrap();
        let mutation = coverage.replace(
            "      - name: Enforce coverage threshold (gwt-core + gwt @ 90%)\n",
            "      - name: Generate lcov\n        if: github.event_name == 'schedule'\n",
        );
        assert_ne!(mutation, coverage);
        write(dir.path(), ".github/workflows/coverage.yml", &mutation);
        assert!(
            derive_pre_pr_for_host(
                dir.path(),
                VerificationHost::Other,
                &required,
                &["cargo test -p gwt --test ci_pre_pr_contract_test".into()],
                &[]
            )
            .is_err(),
            "a display name must not permit skipping the required coverage threshold"
        );
    }

    #[test]
    fn pre_pr_refuses_advisory_required_rust_aggregation() {
        let dir = tempfile::tempdir().unwrap();
        pre_pr_fixture(dir.path());
        let original =
            std::fs::read_to_string(dir.path().join(".github/workflows/test.yml")).unwrap();
        write(
            dir.path(),
            ".github/workflows/test.yml",
            &original.replace(
                "    name: Test (Rust)\n",
                "    name: Test (Rust)\n    continue-on-error: true\n",
            ),
        );
        let result = derive_pre_pr_for_host(
            dir.path(),
            VerificationHost::Other,
            &[
                "Test (Rust)".into(),
                "Clippy & Rustfmt".into(),
                "coverage / Rust Coverage".into(),
            ],
            &["cargo test -p gwt --test ci_pre_pr_contract_test".into()],
            &[],
        );
        assert!(
            result.is_err(),
            "an advisory required guard permits auto-merge on test failure: {result:?}"
        );
    }

    #[test]
    fn pre_pr_keeps_explicit_checks_for_trivial_changes() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let command = "cargo test -p gwt --test ci_pre_pr_contract_test".to_string();
        let plan = derive_pre_pr_for_host(
            dir.path(),
            VerificationHost::Other,
            &[],
            &[],
            std::slice::from_ref(&command),
        )
        .unwrap();
        assert_eq!(plan.commands, vec![command]);
        assert_eq!(plan.trivial_reason, None);
    }

    #[test]
    fn pre_pr_preserves_full_matrix_in_other_projects() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "crates/project/src/lib.rs", "// change\n");
        assert_eq!(
            derive_pre_pr_for_host(dir.path(), VerificationHost::Other, &[], &[], &[]).unwrap(),
            derive_for_host(dir.path(), VerificationHost::Other).unwrap()
        );
    }

    #[test]
    fn pre_pr_keeps_node_runner_outside_required_ci() {
        let dir = tempfile::tempdir().unwrap();
        pre_pr_fixture(dir.path());
        git(dir.path(), &["add", "."]);
        git(
            dir.path(),
            &["commit", "-qm", "test: existing pre-pr workflow"],
        );
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        write(dir.path(), "scripts/tool.mjs", "export {};\n");
        write(dir.path(), "scripts/tool.test.mjs", "import 'node:test';\n");
        let plan = derive_pre_pr_for_host(
            dir.path(),
            VerificationHost::Windows,
            &[
                "Test (Rust)".into(),
                "Clippy & Rustfmt".into(),
                "coverage / Rust Coverage".into(),
            ],
            &["git --version".into()],
            &[],
        )
        .unwrap();
        assert!(
            plan.commands
                .contains(&r#"node --test "scripts/tool.test.mjs""#.into()),
            "{plan:?}"
        );
    }

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
                CI_RUSTDOC_GATE.to_string(),
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

    #[test]
    fn frontend_detection_inspects_integration_branch_changes() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        git(dir.path(), &["checkout", "-q", "-B", "develop"]);
        write(dir.path(), "README.md", "# readme");
        assert!(!has_frontend_changes(dir.path()).unwrap());

        write(dir.path(), "crates/gwt/web/styles/test.css", "body {}\n");
        assert!(has_frontend_changes(dir.path()).unwrap());
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "feat: frontend fixture"]);
        assert!(has_frontend_changes(dir.path()).unwrap());
        assert_eq!(
            derive(dir.path()).unwrap().trivial_reason,
            Some(TrivialReason::IntegrationBranch)
        );

        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(dir.path(), &["config", "diff.renames", "true"]);
        git(
            dir.path(),
            &["mv", "crates/gwt/web/styles/test.css", "archived-style.txt"],
        );
        assert!(
            has_frontend_changes(dir.path()).unwrap(),
            "a staged rename must retain the removed frontend surface"
        );
        git(
            dir.path(),
            &["commit", "-qm", "chore: archive frontend fixture"],
        );
        assert!(
            has_frontend_changes(dir.path()).unwrap(),
            "a committed rename must retain the removed frontend surface"
        );
    }

    /// Issue #4510 AC-2: a frontend *test* file exercises the UI, it never
    /// renders one. Counting `*.spec.ts` as a UI surface made a PR whose whole
    /// diff was one Playwright spec demand a human visual check that had
    /// nothing to look at (PR #4374). The verification matrix still treats the
    /// same path as frontend — the Playwright suite must run — so only the
    /// Ready-handoff question changes here.
    #[test]
    fn frontend_test_only_changes_are_not_a_ui_surface() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());

        write(
            dir.path(),
            "crates/gwt/playwright/tests/pane-close-latency-live.spec.ts",
            "test('pane close', async () => {});\n",
        );
        assert!(
            !has_frontend_changes(dir.path()).unwrap(),
            "a Playwright spec renders no UI of its own"
        );
        write(
            dir.path(),
            "crates/gwt/web/__tests__/kanban.test.js",
            "test('kanban', () => {});\n",
        );
        assert!(
            !has_frontend_changes(dir.path()).unwrap(),
            "a web unit test renders no UI of its own"
        );
        // The matrix is unchanged — the suite that covers these paths still
        // runs — but the surface declares itself as test-only so the Ready
        // handoff does not raise a visual gate over it.
        let plan = derive_for_host(dir.path(), VerificationHost::Other).unwrap();
        assert!(
            plan.commands
                .contains(&package_test_command_for("gwt", VerificationHost::Other)),
            "test-only frontend changes still run the gwt package gate: {:?}",
            plan.commands
        );
        assert!(
            plan.surfaces.contains(&"frontend-tests".to_string())
                && !plan.surfaces.contains(&"frontend".to_string()),
            "{:?}",
            plan.surfaces
        );

        write(dir.path(), "crates/gwt/web/app.js", "export const x = 1;\n");
        assert!(
            has_frontend_changes(dir.path()).unwrap(),
            "a real UI module is still a UI surface"
        );

        let committed = tempfile::tempdir().unwrap();
        fixture(committed.path());
        write(
            committed.path(),
            "crates/gwt/playwright/tests/live.spec.ts",
            "test('live', async () => {});\n",
        );
        write(
            committed.path(),
            "crates/gwt/web/styles/tokens.css",
            ":root {}\n",
        );
        git(committed.path(), &["add", "."]);
        git(committed.path(), &["commit", "-qm", "feat: ui and spec"]);
        assert!(
            has_frontend_changes(committed.path()).unwrap(),
            "a spec alongside a stylesheet keeps the stylesheet's UI surface"
        );
    }

    /// Issue #4637: a script outside `crates/gwt/web/` renders nothing, so
    /// its extension alone must not raise the visual gate — while any change
    /// under `crates/gwt/web/` still does, alone or mixed with such a script.
    #[test]
    fn scripts_outside_the_web_tree_are_not_a_ui_surface() {
        let cases: [(&[&str], bool); 5] = [
            (&["scripts/foo.mjs"], false),
            (&["scripts/foo.test.mjs"], false),
            (&["scripts/foo.test.cjs"], false),
            (&["crates/gwt/web/app.js"], true),
            (&["crates/gwt/web/app.js", "scripts/foo.mjs"], true),
        ];
        for (changed, ui) in cases {
            let dir = tempfile::tempdir().unwrap();
            fixture(dir.path());
            for rel in changed {
                write(dir.path(), rel, "export const x = 1;\n");
            }
            assert_eq!(has_frontend_changes(dir.path()).unwrap(), ui, "{changed:?}");
            let surfaces = derive_for_host(dir.path(), VerificationHost::Other)
                .unwrap()
                .surfaces;
            assert_eq!(
                surfaces.contains(&"frontend".to_string()),
                ui,
                "{changed:?}: {surfaces:?}"
            );
        }
    }

    #[test]
    fn frontend_detection_refuses_unknown_git_or_base() {
        let dir = tempfile::tempdir().unwrap();
        assert!(has_frontend_changes(dir.path()).is_err());

        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        write(dir.path(), "crates/gwt/web/styles/test.css", "body {}\n");
        let error = has_frontend_changes(dir.path()).unwrap_err();
        assert!(error.contains("merge-base"), "{error}");
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

    /// The parsed workflow document.
    fn workflow_doc(workflow: &str) -> serde_yaml::Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root above crates/gwt")
            .join(".github/workflows")
            .join(workflow);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        serde_yaml::from_str(&text).expect("workflow is valid YAML")
    }

    /// Every step of the named job, in step order.
    fn workflow_job_steps(workflow: &str, job: &str) -> Vec<serde_yaml::Value> {
        let doc = workflow_doc(workflow);
        doc["jobs"][job]["steps"]
            .as_sequence()
            .unwrap_or_else(|| panic!("{workflow} job `{job}` has steps"))
            .clone()
    }

    /// Every `run:` script the named job executes, in step order.
    fn workflow_job_runs(workflow: &str, job: &str) -> Vec<String> {
        workflow_job_steps(workflow, job)
            .iter()
            .filter_map(|step| Some(step.get("run")?.as_str()?.to_string()))
            .collect()
    }

    /// The named job's single-line `run:` script rewritten with its `env:`
    /// mapping as leading `KEY=value` assignments — the shape derivation
    /// stores and `verify.run` executes (#3698). A step whose environment
    /// CI supplies out-of-band is otherwise invisible to
    /// [`workflow_job_runs`], which is how the rustdoc gate's `-D warnings`
    /// stayed out of the derived matrix.
    fn workflow_env_prefixed_run(step: &serde_yaml::Value) -> Option<String> {
        let run = step.get("run")?.as_str()?.trim();
        assert!(
            !run.contains('\n'),
            "env-prefixed reconstruction only models single-command steps: {run}"
        );
        let mut parts: Vec<String> = step
            .get("env")
            .and_then(|env| env.as_mapping())
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| {
                let value = value.as_str()?;
                let quoted = if value.is_empty() || value.contains(char::is_whitespace) {
                    format!("\"{value}\"")
                } else {
                    value.to_string()
                };
                Some(format!("{}={quoted}", key.as_str()?))
            })
            .collect();
        parts.push(run.to_string());
        Some(parts.join(" "))
    }

    /// Every `run:` script declared by any job in `workflow` whose runner
    /// image starts with `os` — the jobs that actually compile that platform's
    /// `#[cfg(target_os = ...)]` code.
    fn workflow_runs_on_os(workflow: &str, os: &str) -> Vec<String> {
        let doc = workflow_doc(workflow);
        doc["jobs"]
            .as_mapping()
            .unwrap_or_else(|| panic!("{workflow} declares jobs"))
            .values()
            .filter(|job| {
                job.get("runs-on")
                    .and_then(serde_yaml::Value::as_str)
                    .is_some_and(|runner| runner.starts_with(os))
            })
            .flat_map(|job| {
                job.get("steps")
                    .and_then(serde_yaml::Value::as_sequence)
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|step| Some(step.get("run")?.as_str()?.to_string()))
            .collect()
    }

    /// Every Rust test invocation the job runs, one per script line.
    fn workflow_cargo_tests(workflow: &str, job: &str) -> Vec<String> {
        workflow_job_runs(workflow, job)
            .iter()
            .flat_map(|script| script.lines().collect::<Vec<_>>())
            .map(str::trim)
            .filter(|line| line.contains("cargo test") || line.contains("cargo nextest run"))
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

    // #4968 AC-1: library-only selection silently omitted app_runtime and
    // gwtd unit tests. Binary harnesses do not rebuild the regular binaries.
    #[test]
    fn windows_derivation_includes_gwt_binary_unit_tests() {
        let plan = derive_on(
            VerificationHost::Windows,
            &["crates/gwt/src/app_runtime.rs"],
        );
        assert!(
            plan.commands.contains(
                &"cargo test -p gwt --lib --bins --all-features -- --test-threads=1".to_string()
            ),
            "{:?}",
            plan.commands
        );
        assert!(
            !plan
                .commands
                .contains(&"cargo test -p gwt --all-features".to_string()),
            "the full gate builds the running controller on Windows: {:?}",
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
    #[test]
    fn windows_nextest_profile_preserves_targets_and_doctests() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(
            dir.path(),
            ".config/nextest.toml",
            "[profile.gwt-verify]\nretries = 0\n",
        );
        write(dir.path(), "crates/gwt/src/lib.rs", "");
        write(dir.path(), "crates/gwt-core/src/lib.rs", "");
        let plan = derive_for_host(dir.path(), VerificationHost::Windows).unwrap();
        assert!(
            plan.commands.contains(
                &"cargo nextest run -p gwt --lib --bins --all-features --profile gwt-verify --retries 0"
                    .to_string()
            ),
            "{plan:?}"
        );
        assert!(
            plan.commands.contains(
                &"cargo nextest run -p gwt-core --all-features --profile gwt-verify --retries 0"
                    .to_string()
            ),
            "{plan:?}"
        );
        assert!(
            plan.commands
                .contains(&"cargo test -p gwt-core --all-features --doc".to_string()),
            "{plan:?}"
        );
        assert!(!plan
            .commands
            .iter()
            .any(|command| command.contains("-p gwt --all-features --doc")));
        assert!(!plan
            .commands
            .iter()
            .any(|command| command.contains("--test-threads=1")));
        assert!(plan.unsupported_reason().is_none(), "{plan:?}");
        let other = derive_for_host(dir.path(), VerificationHost::Other).unwrap();
        assert!(other
            .commands
            .contains(&"cargo test -p gwt --all-features".to_string()));
        assert!(
            other
                .surfaces
                .contains(&"unsupported(.config/nextest.toml)".to_string()),
            "{other:?}"
        );
        write(dir.path(), ".config/nextest.toml", "[invalid config\n");
        let invalid = derive_for_host(dir.path(), VerificationHost::Windows).unwrap();
        assert!(
            invalid
                .surfaces
                .contains(&"unsupported(.config/nextest.toml)".to_string()),
            "{invalid:?}"
        );
    }

    // #4968 / #4182: integration targets build the regular binaries and
    // relink the running controller. Library and binary unit harnesses do not.
    #[test]
    fn windows_derivation_never_relinks_the_running_gwtd() {
        for files in [
            vec!["crates/gwt/src/main.rs"],
            vec!["Cargo.toml"],
            vec!["scripts/release.sh"],
        ] {
            for command in cargo_tests(&derive_on(VerificationHost::Windows, &files)) {
                for target in ["--all-targets", "--tests", "--test "] {
                    assert!(
                        !command.contains(target),
                        "{files:?}: `{command}` builds integration targets ({target}) and would \
                         relink the running gwtd"
                    );
                }
                assert!(
                    command.contains(" --lib "),
                    "{files:?}: `{command}` does not explicitly select unit targets"
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
        // twice: once with `--no-run`, once for real. The job left PR CI for
        // the nightly schedule (#4134 AC-1) without changing its gate.
        let gwt_gate = CI_WINDOWS_RUST_TEST_GATE.replace("--workspace", "-p gwt");
        // Local verification includes gwtd's unit harness as well as gwt's.
        let nightly_gate = gwt_gate.replace("--bins", "--bin gwt");
        assert_eq!(
            workflow_cargo_tests("nightly.yml", "test-windows-default-parallel"),
            vec![format!("{nightly_gate} --no-run"), nightly_gate.clone()],
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

    // The controller package excludes integration targets on Windows;
    // every other package keeps CI's full gate, integration tests included.
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

    // #3640 AC-3 / AC-4: preserve the same package/target/feature coverage.
    // #3845 splits this repository's CI gate into isolated nextest tests and
    // rustdoc tests. Its AGENTS.md registers that explicit plan; the portable
    // default still uses cargo test, which includes both families.
    #[test]
    fn derived_rust_matrix_tracks_the_ci_rust_gate() {
        assert_eq!(
            workflow_cargo_tests("test.yml", "test"),
            vec![
                "cargo nextest run --workspace --all-features --test-threads=1".to_string(),
                "cargo test --workspace --all-features --doc".to_string(),
                // The xvfb `--ignored` real-binary family stays CI-owned:
                // it needs a display server, so it is deliberately outside
                // the locally derived matrix.
                "dbus-run-session -- xvfb-run -a cargo test -p gwt --all-features --test stable_server_port --test project_restore_integration --test open_cli_integration -- --ignored --test-threads=1 --nocapture"
                    .to_string(),
            ],
            "CI's Rust test coverage changed — reconcile its explicit plan and the default (#3640, #3845)"
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

    // #3698 AC-2: the rustdoc gate carries `-D warnings` in the step's
    // `env:` mapping, so the #3640 drift check — which only reads `run:` —
    // could not see it, and derivation shipped without a rustdoc gate at
    // all. Pin the reconstructed env-prefixed command so neither half can
    // drift away unnoticed.
    #[test]
    fn derived_rustdoc_gate_tracks_the_ci_rustdoc_gate() {
        let ci_gate = workflow_job_steps("lint.yml", "lint")
            .iter()
            .filter_map(workflow_env_prefixed_run)
            .find(|run| run.contains("cargo doc"))
            .expect("CI's lint gate no longer runs `cargo doc` — update derivation (#3698)");
        assert_eq!(
            ci_gate, CI_RUSTDOC_GATE,
            "CI's rustdoc gate changed — update verify.plan derivation with it (#3698)"
        );
        let plan = derive_for(&["crates/gwt-core/src/lib.rs"]);
        assert!(
            plan.commands.contains(&CI_RUSTDOC_GATE.to_string()),
            "a Rust change must derive CI's rustdoc gate (#3698): {plan:?}"
        );
    }

    // #4522: the gate above is the command every macOS agent has to pass
    // before it can deliver, but the ubuntu and windows clippy jobs never
    // compile `#[cfg(target_os = "macos")]` code, so they cannot report a
    // lint violation hiding behind it. `fsevent-sys` 5.2.0 deprecated its
    // whole C API, CI stayed green through the bump, and `gwt-core` stopped
    // compiling under `-D warnings` on every macOS host at once (#4396 and
    // #3752 each lost hours to it). CI's green has to mean what the local
    // gate means, on the platform the work is done on.
    #[test]
    fn ci_runs_the_clippy_gate_on_macos() {
        let runs = workflow_runs_on_os("lint.yml", "macos");
        assert!(
            runs.iter().any(|run| run.trim() == CI_CLIPPY_GATE),
            "no macOS job in lint.yml runs `{CI_CLIPPY_GATE}`, so a clippy \
             violation behind `#[cfg(target_os = \"macos\")]` passes CI and \
             stops every macOS agent instead (#4522). macOS steps found: \
             {runs:?}"
        );
    }

    // Unknown surfaces retain the fallback matrix for diagnosis, but mark
    // the uncovered path so automatic registration cannot silently pass.
    #[test]
    fn unknown_surface_gets_conservative_default() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "scripts/release.sh", "#!/bin/sh\n");
        write(dir.path(), ".github/workflows/custom.yml", "jobs: {}\n");

        let plan = derive_for_host(dir.path(), VerificationHost::Other).unwrap();
        assert!(plan.commands.contains(&CI_CLIPPY_GATE.to_string()));
        assert!(plan
            .commands
            .contains(&"cargo test -p gwt --all-features".to_string()));
        assert!(plan.surfaces.contains(&"other".to_string()));
        assert!(plan
            .surfaces
            .contains(&"unsupported(scripts/release.sh)".to_string()));
        assert!(
            plan.surfaces
                .contains(&"unsupported(.github/workflows/custom.yml)".to_string()),
            "{plan:?}"
        );
    }

    #[test]
    fn node_scripts_derive_their_existing_tests_without_frontend_classification() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(
            dir.path(),
            "scripts/coverage-summary.test.mjs",
            "import 'node:test';\n",
        );
        write(
            dir.path(),
            "scripts/tool.test.js",
            "require('node:test');\n",
        );
        git(dir.path(), &["add", "."]);
        git(
            dir.path(),
            &["commit", "-qm", "test: existing Node runners"],
        );
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        write(dir.path(), "scripts/coverage-summary.mjs", "export {};\n");
        write(dir.path(), "scripts/tool.js", "module.exports = {};\n");

        let plan = derive_for_host(dir.path(), VerificationHost::Windows).unwrap();
        assert!(
            plan.commands.contains(
                &r#"node --test "scripts/coverage-summary.test.mjs" "scripts/tool.test.js""#
                    .to_string()
            ),
            "{plan:?}"
        );
        assert!(
            plan.surfaces.contains(&"node-tests".to_string()),
            "{plan:?}"
        );
        assert!(
            !plan
                .surfaces
                .iter()
                .any(|surface| surface.starts_with("frontend")),
            "{plan:?}"
        );
    }

    #[test]
    fn node_script_without_tests_is_an_unsupported_surface() {
        let plan = derive_for(&["scripts/no-tests.mjs", "crates/gwt-core/src/lib.rs"]);
        assert!(
            plan.surfaces
                .contains(&"unsupported(scripts/no-tests.mjs)".to_string()),
            "{plan:?}"
        );
        assert!(plan
            .commands
            .contains(&"cargo test -p gwt-core --all-features".to_string()));
    }
}
