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
//! The derived plan is a DEFAULT, not a cage: explicit `verify.plan`
//! commands stay supported, and the recorded plan carries `derived: true`
//! so downstream review can tell the two apart. Acceptance-scenario-driven
//! derivation and plan floor policies remain follow-ups (T-130 full).

use std::{collections::BTreeSet, path::Path};

use gwt_core::process::hidden_command;

/// A derived verification plan: the matrix plus the surface classification
/// that produced it (echoed to the agent for transparency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedPlan {
    pub commands: Vec<String>,
    pub surfaces: Vec<String>,
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
fn integration_merge_base(worktree: &Path) -> Result<String, String> {
    for base_ref in ["origin/develop", "origin/main", "origin/HEAD"] {
        if let Some(base) = git_lines(worktree, &["merge-base", base_ref, "HEAD"])
            .into_iter()
            .next()
        {
            return Ok(base);
        }
    }
    Err(
        "verify.plan derive cannot resolve an integration base (origin/develop, origin/main,          origin/HEAD) — committed branch changes would be invisible to the derived matrix.          Fetch the integration branch or pass explicit params.commands"
            .to_string(),
    )
}

/// Collect the changed paths: the committed span against the integration
/// merge-base, uncommitted changes against HEAD, and untracked files. gwt
/// bookkeeping under `.gwt/` and `tasks/` never counts as a surface.
fn changed_paths(worktree: &Path) -> Result<Vec<String>, String> {
    // On the integration branch itself the committed span is unattributable
    // (merge-base == HEAD hides already-pushed work) — refuse rather than
    // derive a silently weak matrix.
    let head_branch = git_lines(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
        .into_iter()
        .next()
        .unwrap_or_default();
    if matches!(head_branch.as_str(), "develop" | "main" | "master") {
        return Err(format!(
            "verify.plan derive cannot attribute committed work on the integration branch              '{head_branch}' (already-pushed commits are indistinguishable from the base) —              pass explicit params.commands"
        ));
    }
    let base = integration_merge_base(worktree)?;
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
/// `Err` when nothing changed (an empty derived plan would make coverage
/// vacuously satisfied) or when the worktree is not a git checkout.
pub fn derive(worktree: &Path) -> Result<DerivedPlan, String> {
    if git_lines(worktree, &["rev-parse", "--git-dir"]).is_empty() {
        return Err("verify.plan derive requires a git worktree".to_string());
    }
    let paths = changed_paths(worktree)?;
    if paths.is_empty() {
        return Err(
            "no changed surfaces detected (merge-base, HEAD, untracked) — nothing to derive; \
             pass explicit params.commands if the matrix truly differs"
                .to_string(),
        );
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
            "changed paths resolve to no runnable matrix (e.g. deletions only) — pass explicit              params.commands"
                .to_string(),
        );
    }

    Ok(DerivedPlan { commands, surfaces })
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
}
