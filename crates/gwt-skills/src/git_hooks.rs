//! Materialize the Git hook directory `core.hooksPath` points at.
//!
//! Issue #4339: the repository configures `core.hooksPath = .husky/_`, but that
//! directory is generated — it is never part of a clone or a `git worktree add`.
//! Every fresh worktree therefore inherited a `core.hooksPath` aimed at an
//! empty path, and Git silently ran no hook at all: commitlint, the commit-time
//! backend tests, and the pre-push Clippy/coverage gate were all bypassed
//! without a single warning.
//!
//! gwt owns worktree materialization, so it also owns making that directory
//! real. Each required hook becomes a small POSIX shim that forwards to the
//! tracked source next to the hook directory, and the generated directory
//! ignores itself the way `husky` does, so materialization never adds a tracked
//! diff.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gwt_core::process::hidden_command;

/// Marks a hook file as gwt-generated so re-materialization may rewrite it and
/// a foreign hook (a real `husky install`, a user's own script) is left alone.
const MANAGED_HOOK_MARKER: &str = "gwt-managed-git-hook";

/// Client-side hooks gwt materializes when a tracked source exists for them.
const KNOWN_GIT_HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "post-rewrite",
    "pre-auto-gc",
];

/// One hook that must exist inside the configured hook directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHookPlanEntry {
    /// Hook name, e.g. `commit-msg`.
    pub name: String,
    /// Tracked hook script this hook forwards to.
    pub source: PathBuf,
    /// Path Git actually executes.
    pub target: PathBuf,
}

/// What `core.hooksPath` requires for one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHookPlan {
    /// Directory `core.hooksPath` resolves to.
    pub hooks_dir: PathBuf,
    /// Hooks that have a tracked source to forward to.
    pub hooks: Vec<GitHookPlanEntry>,
}

/// Resolve what the worktree's `core.hooksPath` requires, or `None` when gwt
/// owns nothing here.
///
/// gwt owns the hook directory only when it lives inside the worktree and the
/// tracked hook scripts sit right next to it — the `.husky/_` ← `.husky/<hook>`
/// shape. A repository on the Git default resolves to `.git/hooks`, whose
/// neighbouring directory holds no hook script, so it plans nothing.
pub fn plan_managed_git_hooks(worktree: &Path) -> Option<GitHookPlan> {
    let hooks_dir = effective_hooks_dir(worktree)?;
    let source_dir = hooks_dir.parent()?;
    if !is_inside(worktree, &hooks_dir) || !is_inside(worktree, source_dir) {
        return None;
    }

    let mut hooks = Vec::new();
    for name in KNOWN_GIT_HOOKS {
        let source = source_dir.join(name);
        if source.is_file() {
            hooks.push(GitHookPlanEntry {
                name: (*name).to_string(),
                target: hooks_dir.join(name),
                source,
            });
        }
    }

    (!hooks.is_empty()).then_some(GitHookPlan { hooks_dir, hooks })
}

/// Hooks that `core.hooksPath` promises but the filesystem does not deliver.
///
/// A non-empty result means Git is silently skipping those hooks.
pub fn missing_managed_git_hooks(worktree: &Path) -> Vec<PathBuf> {
    let Some(plan) = plan_managed_git_hooks(worktree) else {
        return Vec::new();
    };
    plan.hooks
        .into_iter()
        .filter(|hook| !hook_is_runnable(&hook.target))
        .map(|hook| hook.target)
        .collect()
}

/// Make every hook `core.hooksPath` promises exist and be runnable.
///
/// Returns the hooks written. Foreign hook files are preserved: only a missing
/// hook or a gwt-generated one is written.
pub fn materialize_managed_git_hooks(worktree: &Path) -> io::Result<Vec<PathBuf>> {
    let Some(plan) = plan_managed_git_hooks(worktree) else {
        return Ok(Vec::new());
    };

    fs::create_dir_all(&plan.hooks_dir)?;
    // The generated directory ignores itself, including this file, so a
    // materialized worktree stays free of tracked diffs.
    let self_ignore = plan.hooks_dir.join(".gitignore");
    if !self_ignore.exists() {
        fs::write(&self_ignore, "*\n")?;
    }

    let mut written = Vec::new();
    for hook in plan.hooks {
        // Never rewrite a hook gwt did not generate: a real `husky install` or
        // a user's own script owns that file. A foreign hook that Git cannot
        // run stays reported by [`missing_managed_git_hooks`] instead.
        if hook.target.exists() && !is_managed_hook(&hook.target) {
            continue;
        }
        let shim = shim_for(&hook.name);
        if hook_is_runnable(&hook.target)
            && fs::read_to_string(&hook.target).is_ok_and(|current| current == shim)
        {
            continue;
        }
        fs::write(&hook.target, shim)?;
        make_executable(&hook.target)?;
        written.push(hook.target);
    }
    Ok(written)
}

/// A hook shim forwards to the tracked source one directory above it, so a
/// later edit of the tracked hook takes effect without re-materialization.
fn shim_for(name: &str) -> String {
    format!(
        "#!/usr/bin/env sh\n\
         # {MANAGED_HOOK_MARKER}: generated by gwt, do not edit (Issue #4339).\n\
         hook_source=\"$(dirname \"$0\")/../{name}\"\n\
         [ -f \"$hook_source\" ] || exit 0\n\
         exec sh \"$hook_source\" \"$@\"\n"
    )
}

fn is_managed_hook(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|content| content.contains(MANAGED_HOOK_MARKER))
}

fn hook_is_runnable(path: &Path) -> bool {
    path.is_file() && is_executable(path)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The directory Git runs hooks from, honouring `core.hooksPath`.
///
/// `git rev-parse --git-path hooks` is Git's own answer, so a relative
/// `core.hooksPath` lands on the worktree root exactly the way Git runs it.
/// One `git` call keeps this affordable: health projects it once per Work row
/// (#4172).
fn effective_hooks_dir(worktree: &Path) -> Option<PathBuf> {
    let resolved = git_output(worktree, &["rev-parse", "--git-path", "hooks"])?;
    if resolved.is_empty() {
        return None;
    }
    let resolved = PathBuf::from(resolved);
    Some(if resolved.is_absolute() {
        resolved
    } else {
        worktree.join(resolved)
    })
}

fn git_output(worktree: &Path, args: &[&str]) -> Option<String> {
    let output = hidden_command("git")
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn is_inside(worktree: &Path, candidate: &Path) -> bool {
    let worktree = dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let candidate = deepest_existing_canonical(candidate);
    candidate.starts_with(&worktree) && candidate != worktree
}

/// Canonicalize what exists and keep the rest, so a not-yet-created hook
/// directory still compares against the worktree by its real path.
fn deepest_existing_canonical(path: &Path) -> PathBuf {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        if let Ok(mut canonical) = dunce::canonicalize(ancestor) {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        let (Some(name), Some(parent)) = (ancestor.file_name(), ancestor.parent()) else {
            return path.to_path_buf();
        };
        suffix.push(name.to_os_string());
        ancestor = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(dir: &Path, args: &[&str]) -> std::process::Output {
        hidden_command("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("run git")
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let output = run_git(dir, args);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A repo whose `core.hooksPath` points at a generated directory that a
    /// clone or `git worktree add` never creates.
    fn repo_with_generated_hooks_path() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git_ok(root, &["init"]);
        git_ok(root, &["config", "user.email", "test@example.com"]);
        git_ok(root, &["config", "user.name", "Test User"]);
        git_ok(root, &["config", "core.hooksPath", ".husky/_"]);

        fs::create_dir_all(root.join(".husky")).expect("create .husky");
        fs::write(
            root.join(".husky/commit-msg"),
            "#!/usr/bin/env sh\ngrep -q '^feat:' \"$1\" || exit 1\n",
        )
        .expect("write commit-msg");
        fs::write(
            root.join(".husky/pre-commit"),
            "#!/usr/bin/env sh\nexit 0\n",
        )
        .expect("write pre-commit");
        git_ok(root, &["add", "."]);
        // The configured hook directory does not exist yet, so Git runs no hook
        // here — which is exactly the bug this module fixes.
        git_ok(root, &["commit", "-q", "-m", "feat: seed"]);
        dir
    }

    #[test]
    fn missing_hooks_are_reported_before_materialization() {
        let repo = repo_with_generated_hooks_path();
        let root = repo.path();

        let missing = missing_managed_git_hooks(root);

        assert_eq!(
            missing,
            vec![
                root.join(".husky/_/pre-commit"),
                root.join(".husky/_/commit-msg"),
            ],
            "an empty core.hooksPath directory must be reported, not silently skipped"
        );
    }

    #[test]
    fn materialization_makes_git_run_the_tracked_hook() {
        let repo = repo_with_generated_hooks_path();
        let root = repo.path();

        materialize_managed_git_hooks(root).expect("materialize hooks");

        assert!(
            missing_managed_git_hooks(root).is_empty(),
            "every configured hook should exist after materialization"
        );
        fs::write(root.join("file.txt"), "content").expect("write file");
        git_ok(root, &["add", "file.txt"]);
        let rejected = run_git(root, &["commit", "-m", "bad message"]);
        assert!(
            !rejected.status.success(),
            "commit-msg must reject a non-conforming message: {}",
            String::from_utf8_lossy(&rejected.stdout)
        );
        git_ok(root, &["commit", "-m", "feat: good message"]);
    }

    #[test]
    fn materialization_adds_no_tracked_diff() {
        let repo = repo_with_generated_hooks_path();
        let root = repo.path();

        materialize_managed_git_hooks(root).expect("materialize hooks");

        let status = run_git(root, &["status", "--porcelain"]);
        assert!(
            String::from_utf8_lossy(&status.stdout).trim().is_empty(),
            "generated hooks must stay ignored: {}",
            String::from_utf8_lossy(&status.stdout)
        );
    }

    #[test]
    fn foreign_hook_files_are_preserved() {
        let repo = repo_with_generated_hooks_path();
        let root = repo.path();
        let foreign = root.join(".husky/_/commit-msg");
        fs::create_dir_all(foreign.parent().expect("hooks dir")).expect("create hooks dir");
        fs::write(&foreign, "#!/usr/bin/env sh\n# husky\nexit 0\n").expect("write foreign hook");
        make_executable(&foreign).expect("chmod foreign hook");

        materialize_managed_git_hooks(root).expect("materialize hooks");

        assert!(
            fs::read_to_string(&foreign)
                .expect("read foreign hook")
                .contains("# husky"),
            "a hook gwt did not generate must not be overwritten"
        );
    }

    #[test]
    fn repository_without_hooks_path_is_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git_ok(root, &["init"]);

        assert_eq!(plan_managed_git_hooks(root), None);
        assert!(materialize_managed_git_hooks(root)
            .expect("no-op materialization")
            .is_empty());
    }
}
