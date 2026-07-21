//! Verification-plan derivation from changed surfaces (SPEC-3248 full
//! T-130 core).
//!
//! `verify.plan` with `params.derive:true` classifies the worktree's
//! changed files — branch changes against the `origin/develop` merge-base
//! when available, plus uncommitted changes and untracked files — into
//! surfaces and derives the verification matrix from them, instead of
//! trusting the agent to hand-pick commands:
//!
//! - **rust** (`crates/<name>/…`, workspace manifests): `cargo fmt --check`,
//!   workspace clippy with `-D warnings`, and `cargo test -p <crate>` per
//!   changed crate (`--lib` for the `gwt` crate — its integration tests
//!   carry live-process families that the CI gate owns).
//! - **skills / guidance** (`.claude/skills/`, `.codex/skills/`): the
//!   `gwt-skills` test suite (managed-asset parity lives there).
//! - **frontend** (`crates/gwt/web/` and js/ts/css/html): the embedded web
//!   contract tests in the `gwt` lib suite.
//! - **docs** (markdown outside the skill trees): `bunx markdownlint-cli2`
//!   over the changed files (AGENTS markdown policy).
//! - **anything else** (scripts, CI config, …): the conservative default —
//!   workspace clippy + the `gwt` lib suite.
//!
//! Ordinary changed surfaces still support explicit `verify.plan` commands.
//! A bookkeeping-only recovery is deliberately stricter: derivation owns the
//! exact non-vacuous floor-plus-blocker-requirements plan and binds its base,
//! classifier version, requirement hash, and worktree fingerprint.

use std::{collections::BTreeSet, path::Path};

use gwt_core::process::hidden_command;
use serde::{Deserialize, Serialize};

use crate::cli::execution_state::{
    self, ExecutionControlRecord, ExecutionControlStatus, RecoveryExecutionRoot,
};

/// Version of the exact, closed repository-root bookkeeping allowlist.
pub const BOOKKEEPING_CLASSIFIER_VERSION: &str = "bookkeeping-v1";

/// Hash-bound provenance for the special bookkeeping-only recovery plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoChangeDerivationEvidence {
    pub integration_base_ref: String,
    pub integration_base: String,
    pub classifier_version: String,
    pub required_recovery_commands_hash: String,
}

/// A derived verification plan: the matrix plus the surface classification
/// that produced it (echoed to the agent for transparency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedPlan {
    pub commands: Vec<String>,
    pub surfaces: Vec<String>,
    pub no_change_evidence: Option<NoChangeDerivationEvidence>,
}

fn git_output(worktree: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let output = hidden_command("git")
        .arg("-C")
        .arg(worktree)
        // Non-ASCII paths must come back verbatim, not quote-escaped —
        // escaped spellings would defeat every classifier and exclusion.
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .output()
        .map_err(|err| format!("failed to run git {}: {err}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn git_lines(worktree: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let output = git_output(worktree, args)?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Read repository paths without trimming, quote decoding, or lossy Unicode
/// conversion. Any such normalization before classification could turn a
/// lookalike path into a bookkeeping allowlist member.
fn git_paths(worktree: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let output = git_output(worktree, args)?;
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path).map(str::to_string).map_err(|_| {
                "verify.plan derive encountered a non-UTF-8 repository path; refusing to classify it lossily"
                    .to_string()
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IntegrationBase {
    reference: String,
    commit: String,
}

/// Resolve the integration base the committed span is diffed against.
/// Fail-closed: without a resolvable base the committed branch work would
/// silently vanish from the matrix, so derivation refuses instead.
fn integration_merge_base(worktree: &Path) -> Result<IntegrationBase, String> {
    for base_ref in ["origin/develop", "origin/main", "origin/HEAD"] {
        let output = hidden_command("git")
            .arg("-C")
            .arg(worktree)
            .args(["merge-base", base_ref, "HEAD"])
            .output()
            .map_err(|err| format!("failed to resolve integration base {base_ref}: {err}"))?;
        if output.status.success() {
            if let Some(commit) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
            {
                return Ok(IntegrationBase {
                    reference: base_ref.to_string(),
                    commit: commit.to_string(),
                });
            }
        }
    }
    Err(
        "verify.plan derive cannot resolve an integration base (origin/develop, origin/main, \
         origin/HEAD) — committed branch changes would be invisible to the derived matrix. \
         Fetch the integration branch or pass explicit params.commands"
            .to_string(),
    )
}

#[derive(Debug, Clone)]
struct ObservedChanges {
    integration_base: IntegrationBase,
    paths: Vec<String>,
}

/// Collect every changed path before classification: the committed span
/// against the integration merge-base, uncommitted changes against HEAD,
/// and non-ignored untracked files.
fn observed_changes(worktree: &Path) -> Result<ObservedChanges, String> {
    // On the integration branch itself the committed span is unattributable
    // (merge-base == HEAD hides already-pushed work) — refuse rather than
    // derive a silently weak matrix.
    let head_branch = git_lines(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .into_iter()
        .next()
        .unwrap_or_default();
    if matches!(head_branch.as_str(), "develop" | "main" | "master") {
        return Err(format!(
            "verify.plan derive cannot attribute committed work on the integration branch \
             '{head_branch}' (already-pushed commits are indistinguishable from the base) — \
             pass explicit params.commands"
        ));
    }
    let integration_base = integration_merge_base(worktree)?;
    let mut paths: BTreeSet<String> = BTreeSet::new();
    paths.extend(git_paths(
        worktree,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            &integration_base.commit,
            "HEAD",
        ],
    )?);
    paths.extend(git_paths(
        worktree,
        &["diff", "--name-only", "--no-renames", "-z", "HEAD"],
    )?);
    paths.extend(git_paths(
        worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    Ok(ObservedChanges {
        integration_base,
        paths: paths.into_iter().collect(),
    })
}

fn has_safe_relative_segments(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_bookkeeping_path(path: &str) -> bool {
    has_safe_relative_segments(path)
        && [".gwt/", "tasks/"].iter().any(|prefix| {
            path.strip_prefix(prefix)
                .is_some_and(|relative| !relative.is_empty())
        })
}

/// Canonical executable proof that the resolved base-to-worktree tracked
/// span contains no non-bookkeeping change. Untracked drift is bound by the
/// plan/run worktree fingerprint and the pre-filter observation pass.
#[must_use]
pub fn no_change_floor_command(integration_base: &str) -> String {
    format!("git diff --quiet {integration_base} -- . \":(exclude).gwt/**\" \":(exclude)tasks/**\"")
}

fn valid_required_recovery_commands(record: &ExecutionControlRecord) -> Result<(), String> {
    if record.content_hash.is_empty()
        || !execution_state::integrity_ok(record)
        || record.required_recovery_commands.is_empty()
        || record.required_recovery_commands_hash.is_empty()
    {
        return Err(
            "Legacy Requirement Gap: the Blocked record has no valid Required Recovery Command Set; same-session recovery cannot infer or append one — use a fresh linked-owner launch"
                .to_string(),
        );
    }
    if record
        .required_recovery_commands
        .iter()
        .any(|required| required.execution_root != RecoveryExecutionRoot::Worktree)
    {
        return Err(
            "Required Recovery Command Set uses an unsupported execution root — use a fresh linked-owner launch"
                .to_string(),
        );
    }
    Ok(())
}

fn blocked_required_command_texts(worktree: &Path) -> Result<Vec<String>, String> {
    let Some(record) = execution_state::load(worktree)
        .map_err(|err| format!("failed to load Execution Control Record: {err}"))?
    else {
        return Ok(Vec::new());
    };
    if record.status != ExecutionControlStatus::Blocked {
        return Ok(Vec::new());
    }
    valid_required_recovery_commands(&record)?;
    Ok(record
        .required_recovery_commands
        .into_iter()
        .map(|required| required.command)
        .collect())
}

fn derive_no_change(worktree: &Path, observed: &ObservedChanges) -> Result<DerivedPlan, String> {
    let Some(record) = execution_state::load(worktree)
        .map_err(|err| format!("failed to load Execution Control Record: {err}"))?
    else {
        return Err(
            "no changed surfaces detected (merge-base, HEAD, untracked) — nothing to derive; pass explicit params.commands if the matrix truly differs"
                .to_string(),
        );
    };
    if record.status != ExecutionControlStatus::Blocked {
        return Err(
            "no changed surfaces detected (merge-base, HEAD, untracked) — the non-vacuous no-change floor is available only for a terminal Blocked recovery"
                .to_string(),
        );
    }
    valid_required_recovery_commands(&record)?;

    let floor = no_change_floor_command(&observed.integration_base.commit);
    let mut commands = vec![floor];
    for required in &record.required_recovery_commands {
        if !commands.contains(&required.command) {
            commands.push(required.command.clone());
        }
    }
    Ok(DerivedPlan {
        commands,
        surfaces: vec![format!(
            "bookkeeping-only({BOOKKEEPING_CLASSIFIER_VERSION})"
        )],
        no_change_evidence: Some(NoChangeDerivationEvidence {
            integration_base_ref: observed.integration_base.reference.clone(),
            integration_base: observed.integration_base.commit.clone(),
            classifier_version: BOOKKEEPING_CLASSIFIER_VERSION.to_string(),
            required_recovery_commands_hash: record.required_recovery_commands_hash,
        }),
    })
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
/// `Err` when nothing changed (an empty derived plan would make coverage
/// vacuously satisfied) or when the worktree is not a git checkout.
pub fn derive(worktree: &Path) -> Result<DerivedPlan, String> {
    if !git_lines(worktree, &["rev-parse", "--git-dir"]).is_ok_and(|lines| !lines.is_empty()) {
        return Err("verify.plan derive requires a git worktree".to_string());
    }
    let observed = observed_changes(worktree)?;
    let paths: Vec<String> = observed
        .paths
        .iter()
        .filter(|path| !is_bookkeeping_path(path))
        .cloned()
        .collect();
    if paths.is_empty() {
        return derive_no_change(worktree, &observed);
    }
    let blocked_required_commands = blocked_required_command_texts(worktree)?;

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
        push_unique(&mut commands, "cargo fmt --check".to_string());
        push_unique(
            &mut commands,
            "cargo clippy --all-targets --all-features -- -D warnings".to_string(),
        );
    }
    if !rust_crates.is_empty() || workspace_rust {
        surfaces.push(format!(
            "rust({})",
            if rust_crates.is_empty() {
                "workspace".to_string()
            } else {
                rust_crates.iter().cloned().collect::<Vec<_>>().join(",")
            }
        ));
        for name in &rust_crates {
            if name == "gwt" {
                push_unique(&mut commands, "cargo test -p gwt --lib".to_string());
            } else {
                push_unique(&mut commands, format!("cargo test -p {name}"));
            }
        }
        if workspace_rust && rust_crates.is_empty() {
            push_unique(&mut commands, "cargo test -p gwt --lib".to_string());
        }
    }
    if skills {
        surfaces.push("skills".to_string());
        push_unique(&mut commands, "cargo test -p gwt-skills".to_string());
    }
    if frontend {
        surfaces.push("frontend".to_string());
        push_unique(&mut commands, "cargo test -p gwt --lib".to_string());
    }
    if other {
        surfaces.push("other".to_string());
        push_unique(&mut commands, "cargo test -p gwt --lib".to_string());
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
        return Err(
            "changed paths resolve to no runnable matrix (e.g. deletions only) — pass explicit \
             params.commands"
                .to_string(),
        );
    }
    // A normal changed-surface matrix proves the code delta, but it cannot
    // substitute for the verifier captured with the blocker. Keep the
    // machine-derived surface order, then append each missing required
    // command in trusted first-seen order.
    for required in blocked_required_commands {
        push_unique(&mut commands, required);
    }

    Ok(DerivedPlan {
        commands,
        surfaces,
        no_change_evidence: None,
    })
}

/// Revalidate a recovery plan against the current repository and immutable
/// blocker snapshot. Ordinary non-bookkeeping plans retain their existing
/// derivation contract; bookkeeping-only recovery requires the exact
/// floor-plus-requirements union and current classifier/base identity.
pub fn validate_recovery_plan(
    worktree: &Path,
    record: &ExecutionControlRecord,
    plan: &crate::cli::verification_record::VerificationPlanRecord,
) -> Result<(), String> {
    let observed = observed_changes(worktree)?;
    // FR-200 applies to every same-session reopen. The special floor below
    // is conditional on no ordinary surface, but a source change must never
    // turn a Legacy Requirement Gap into a recoverable record.
    valid_required_recovery_commands(record)?;
    let ordinary_paths: Vec<&String> = observed
        .paths
        .iter()
        .filter(|path| !is_bookkeeping_path(path))
        .collect();
    if !ordinary_paths.is_empty() {
        if plan.no_change_evidence.is_some() {
            return Err(
                "the no-change proof is stale: a non-bookkeeping surface is now present — derive and run a fresh plan"
                    .to_string(),
            );
        }
        let missing: Vec<&str> = record
            .required_recovery_commands
            .iter()
            .map(|required| required.command.as_str())
            .filter(|required| !plan.commands.iter().any(|command| command == required))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "the derived recovery plan omits {} command(s) from the Required Recovery Command Set — derive and run a fresh plan",
                missing.len()
            ));
        }
        return Ok(());
    }

    let evidence = plan.no_change_evidence.as_ref().ok_or_else(|| {
        "no-change recovery requires the canonical Non-Vacuous No-Change Floor; floor-only, explicit, or substituted plans are not accepted — derive a fresh plan or use a fresh linked-owner launch"
            .to_string()
    })?;
    if evidence.classifier_version != BOOKKEEPING_CLASSIFIER_VERSION {
        return Err(
            "the no-change bookkeeping classifier version is stale — derive a fresh plan"
                .to_string(),
        );
    }
    if evidence.integration_base_ref != observed.integration_base.reference
        || evidence.integration_base != observed.integration_base.commit
    {
        return Err(
            "the resolved integration-base identity changed after no-change derivation — derive a fresh plan"
                .to_string(),
        );
    }
    if evidence.required_recovery_commands_hash != record.required_recovery_commands_hash {
        return Err(
            "the verification plan is not bound to the Blocked record's Required Recovery Command Set — use a fresh linked-owner launch"
                .to_string(),
        );
    }
    let mut expected = vec![no_change_floor_command(&evidence.integration_base)];
    for required in &record.required_recovery_commands {
        if !expected.contains(&required.command) {
            expected.push(required.command.clone());
        }
    }
    if plan.commands != expected {
        return Err(
            "the no-change verification plan is not the exact canonical floor-plus-required-command union — derive and run a fresh plan"
                .to_string(),
        );
    }
    Ok(())
}

/// Revalidate provenance for gates that already have a plan snapshot.
pub fn validate_no_change_plan(
    worktree: &Path,
    plan: &crate::cli::verification_record::VerificationPlanRecord,
) -> Result<(), String> {
    if plan.no_change_evidence.is_none() {
        return Ok(());
    }
    let record = execution_state::load(worktree)
        .map_err(|err| format!("failed to load Execution Control Record: {err}"))?
        .ok_or_else(|| "the no-change plan has no Execution Control Record".to_string())?;
    validate_recovery_plan(worktree, &record, plan)
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

        let plan = derive(dir.path()).unwrap();
        assert_eq!(
            plan.commands,
            vec![
                "cargo fmt --check".to_string(),
                "cargo clippy --all-targets --all-features -- -D warnings".to_string(),
                "cargo test -p gwt --lib".to_string(),
                "cargo test -p gwt-core".to_string(),
                "cargo test -p gwt-skills".to_string(),
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

    // T-130 review fixes: committed branch work counts through the
    // merge-base leg; unresolvable bases and integration branches refuse
    // instead of deriving a silently weak matrix.
    #[test]
    fn committed_branch_changes_join_the_matrix() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "crates/gwt-core/src/lib.rs", "pub fn x() {}");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "feat: core change"]);
        // Only a doc is dirty now — the committed rust must still derive.
        write(dir.path(), "README.md", "# readme");

        let plan = derive(dir.path()).unwrap();
        assert!(
            plan.commands
                .contains(&"cargo test -p gwt-core".to_string()),
            "{:?}",
            plan.commands
        );
        assert!(plan
            .commands
            .contains(&"cargo clippy --all-targets --all-features -- -D warnings".to_string()));
    }

    #[test]
    fn unresolvable_base_and_integration_branch_refuse() {
        // No origin/develop|main|HEAD refs at all → fail closed.
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        git(dir.path(), &["checkout", "-q", "-b", "work/fixture"]);
        write(dir.path(), "README.md", "# readme");
        let err = derive(dir.path()).unwrap_err();
        assert!(err.contains("integration base"), "{err}");

        // Sitting on the integration branch itself → refuse (already-pushed
        // work is indistinguishable from the base).
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(dir.path(), &["checkout", "-q", "-B", "develop"]);
        write(dir.path(), "README.md", "# readme");
        let err = derive(dir.path()).unwrap_err();
        assert!(err.contains("integration branch"), "{err}");
    }

    // Deletions-only change sets refuse rather than derive a vacuous
    // markdownlint run over zero files.
    #[test]
    fn deleted_docs_only_refuses() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "notes.md", "# notes");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "docs: notes"]);
        std::fs::remove_file(dir.path().join("notes.md")).unwrap();

        let err = derive(dir.path()).unwrap_err();
        assert!(err.contains("no runnable matrix"), "{err}");
    }

    // No changes → refuse to derive (an empty plan would satisfy coverage
    // vacuously). Non-git dirs refuse too.
    #[test]
    fn refuses_empty_and_non_git_derivation() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let err = derive(dir.path()).unwrap_err();
        assert!(err.contains("nothing to derive"), "{err}");

        let plain = tempfile::tempdir().unwrap();
        let err = derive(plain.path()).unwrap_err();
        assert!(err.contains("git worktree"), "{err}");
    }

    // Unknown surfaces (scripts, CI config) fall back to the conservative
    // default matrix.
    #[test]
    fn unknown_surface_gets_conservative_default() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), "scripts/release.sh", "#!/bin/sh\n");

        let plan = derive(dir.path()).unwrap();
        assert!(plan
            .commands
            .contains(&"cargo clippy --all-targets --all-features -- -D warnings".to_string()));
        assert!(plan
            .commands
            .contains(&"cargo test -p gwt --lib".to_string()));
        assert!(plan.surfaces.contains(&"other".to_string()));
    }

    #[test]
    fn bookkeeping_plus_production_uses_the_ordinary_matrix() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), ".gwt/work/events.jsonl", "{}\n");
        git(dir.path(), &["add", "-f", ".gwt/work/events.jsonl"]);
        write(
            dir.path(),
            "crates/gwt-core/src/lib.rs",
            "pub fn changed() {}\n",
        );

        let plan = derive(dir.path()).unwrap();
        assert!(plan.no_change_evidence.is_none());
        assert!(plan
            .commands
            .contains(&"cargo test -p gwt-core".to_string()));
        assert!(!plan
            .surfaces
            .iter()
            .any(|surface| surface.starts_with("bookkeeping-only")));
    }

    #[test]
    fn case_lookalike_and_nested_aliases_are_not_bookkeeping() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), ".GWT/work/events.jsonl", "{}\n");
        write(dir.path(), "nested/.gwt/work/events.jsonl", "{}\n");
        write(dir.path(), "tasks-lookalike/todo.md", "- [ ] x\n");
        write(dir.path(), "untracked.bin", "not bookkeeping\n");

        let plan = derive(dir.path()).unwrap();
        assert!(plan.no_change_evidence.is_none());
        assert!(plan.surfaces.contains(&"other".to_string()));
        assert!(plan
            .commands
            .contains(&"cargo test -p gwt --lib".to_string()));
    }

    #[test]
    fn leading_space_bookkeeping_lookalike_is_not_trimmed_into_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        write(dir.path(), " .gwt/work/events.jsonl", "{}\n");

        let plan = derive(dir.path()).unwrap();
        assert!(plan.no_change_evidence.is_none());
        assert!(plan.surfaces.contains(&"other".to_string()));
    }

    #[test]
    fn rename_from_runnable_surface_into_bookkeeping_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());
        write(dir.path(), "README.md", "# before\n");
        git(dir.path(), &["add", "README.md"]);
        git(dir.path(), &["commit", "-qm", "docs: seed readme"]);
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(dir.path(), &["checkout", "-q", "-b", "work/rename"]);
        std::fs::create_dir_all(dir.path().join(".gwt/work")).unwrap();
        git(dir.path(), &["mv", "README.md", ".gwt/work/README.md"]);

        let err = derive(dir.path()).unwrap_err();
        assert!(err.contains("no runnable matrix"), "{err}");
    }
}
