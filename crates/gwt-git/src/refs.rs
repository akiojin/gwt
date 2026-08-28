//! Bulk ref existence lookup helpers.
//!
//! `list_existing_refs` checks whether each of the supplied fully-qualified
//! ref names exists in `repo_path`, using a **single** `git for-each-ref`
//! invocation. This is the primary tool for collapsing the Launch Wizard
//! cold-open path on Windows, where every additional `git.exe` spawn pays a
//! `CreateProcess` + Defender real-time-scan cost of several hundred
//! milliseconds (SPEC-2014 FR-PERF-001 / FR-PERF-002).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use gwt_core::{GwtError, Result};

/// A network-free canonical source snapshot pinned to an immutable Git tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRootTree {
    pub reference: String,
    pub root_tree_oid: String,
}

/// Resolve the first locally available canonical ref to its immutable root
/// tree OID. No fetch is attempted: refresh admission must pin a snapshot of
/// the refs already present on disk.
///
/// An unborn/empty repository returns `Ok(None)` so callers can use the empty
/// base contract and classify every visible record as overlay content.
pub fn resolve_canonical_root_tree(repo_path: &Path) -> Result<Option<CanonicalRootTree>> {
    const CANDIDATES: [&str; 5] = [
        "origin/develop",
        "origin/HEAD",
        "origin/main",
        "origin/master",
        "HEAD",
    ];

    let repo_path = effective_refs_root(repo_path);
    let repository_probe = gwt_core::process::run_git_logged(
        &["rev-parse", "--is-inside-work-tree"],
        Some(&repo_path),
    )
    .map_err(|error| GwtError::Git(format!("rev-parse repository probe: {error}")))?;
    if !repository_probe.status.success() {
        if repo_path.join(".git").exists() {
            let stderr = String::from_utf8_lossy(&repository_probe.stderr)
                .trim()
                .to_string();
            return Err(GwtError::Git(format!(
                "repository metadata is present but unreadable: {stderr}"
            )));
        }
        return Ok(None);
    }
    for reference in CANDIDATES {
        let tree_expression = format!("{reference}^{{tree}}");
        let output = gwt_core::process::run_git_logged(
            &["rev-parse", "--verify", "--quiet", &tree_expression],
            Some(&repo_path),
        )
        .map_err(|error| GwtError::Git(format!("rev-parse {tree_expression}: {error}")))?;
        if !output.status.success() {
            continue;
        }
        let root_tree_oid = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_ascii_lowercase();
        let valid_oid = matches!(root_tree_oid.len(), 40 | 64)
            && root_tree_oid.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !valid_oid {
            return Err(GwtError::Git(format!(
                "rev-parse {tree_expression}: invalid tree oid {root_tree_oid:?}"
            )));
        }
        return Ok(Some(CanonicalRootTree {
            reference: reference.to_string(),
            root_tree_oid,
        }));
    }
    Ok(None)
}

/// Issue #3629 AC-4: the bulk snapshot helpers are handed a *project root*,
/// which for the workspace-home layout is not a git work tree. Resolve the
/// directory the ref enumeration should actually run in — without spawning
/// git for the nested-bare layout — so the snapshot works instead of pushing
/// every consumer into per-branch probe fallbacks.
fn effective_refs_root(repo_path: &Path) -> PathBuf {
    crate::worktree::effective_repo_root(repo_path)
}

/// Return the subset of `candidates` that resolve to existing refs in
/// `repo_path`.
///
/// `candidates` must contain fully-qualified ref names (for example
/// `refs/remotes/origin/develop` or `refs/heads/work/20260513-0315`).
/// The function runs `git for-each-ref --format=%(refname) <ref...>` exactly
/// once and parses the output as the existence set. Non-matching candidates
/// are silently absent from the returned set.
///
/// Returns an empty `HashSet` without spawning git when `candidates` is empty
/// (a `for-each-ref` with no patterns would otherwise list every ref in the
/// repository).
pub fn list_existing_refs(repo_path: &Path, candidates: &[&str]) -> Result<HashSet<String>> {
    let trimmed: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.trim())
        .filter(|candidate| !candidate.is_empty())
        .collect();
    if trimmed.is_empty() {
        return Ok(HashSet::new());
    }

    let mut args: Vec<&str> = vec!["for-each-ref", "--format=%(refname)"];
    args.extend(trimmed.iter().copied());
    let output = gwt_core::process::run_git_logged(&args, Some(repo_path))
        .map_err(|error| GwtError::Git(format!("for-each-ref: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Git(format!("for-each-ref: {stderr}")));
    }

    let existing: HashSet<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    Ok(existing)
}

/// SPEC-2359 W-16 (FR-387): enumerate every `refs/remotes/origin/*` ref with
/// its commit sha in ONE `for-each-ref` spawn. `origin/HEAD` (a symref) is
/// skipped. The pair feeds `blob::events_blob_oids_batch` so the intake can
/// read `events.jsonl` from fetched branches without checking anything out.
pub fn list_origin_refs_with_commit(repo_path: &Path) -> Result<Vec<(String, String)>> {
    let repo_path = effective_refs_root(repo_path);
    let output = gwt_core::process::run_git_logged(
        &[
            "for-each-ref",
            "--format=%(refname)\t%(objectname)",
            "refs/remotes/origin/",
        ],
        Some(&repo_path),
    )
    .map_err(|error| GwtError::Git(format!("for-each-ref origin: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Git(format!("for-each-ref origin: {stderr}")));
    }
    let refs = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (refname, sha) = line.split_once('\t')?;
            let refname = refname.trim();
            let sha = sha.trim();
            if refname.is_empty() || sha.is_empty() || refname == "refs/remotes/origin/HEAD" {
                return None;
            }
            Some((refname.to_string(), sha.to_string()))
        })
        .collect();
    Ok(refs)
}

/// SPEC-2359 W16-4 (FR-391): committer time (unix seconds) of every local
/// branch tip and every `origin/*` tip, in ONE `for-each-ref` spawn. Keys
/// are short ref names (`work/x`, `origin/work/x`). Used as the
/// merge-reference-time proxy for the derived Done classification.
pub fn branch_tip_committer_times(
    repo_path: &Path,
) -> Result<std::collections::HashMap<String, i64>> {
    let repo_path = effective_refs_root(repo_path);
    let output = gwt_core::process::run_git_logged(
        &[
            "for-each-ref",
            "--format=%(refname:short)\t%(committerdate:unix)",
            "refs/heads/",
            "refs/remotes/origin/",
        ],
        Some(&repo_path),
    )
    .map_err(|error| GwtError::Git(format!("for-each-ref tip times: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Git(format!("for-each-ref tip times: {stderr}")));
    }
    let times = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, unix) = line.split_once('\t')?;
            let name = name.trim();
            let unix: i64 = unix.trim().parse().ok()?;
            (!name.is_empty()).then(|| (name.to_string(), unix))
        })
        .collect();
    Ok(times)
}

/// SPEC-3075: the tip commit subject of every local / `origin/*` branch, in ONE
/// `for-each-ref` spawn. Keys are short ref names (`work/x`, `origin/work/x`).
/// This is the "what work was running" signal for historical Workspaces that
/// never recorded a purpose (their title is only the timestamp branch name) —
/// for a conventional-commit repo the tip subject (`feat(...): ...`) is a
/// human-readable summary. Resolved off the hot path and cached, mirroring
/// [`branch_tip_committer_times`] (Issue #2725 keeps git out of the projection
/// build). Branches with an empty subject are skipped.
pub fn branch_tip_subjects(repo_path: &Path) -> Result<std::collections::HashMap<String, String>> {
    let repo_path = effective_refs_root(repo_path);
    let output = gwt_core::process::run_git_logged(
        &[
            "for-each-ref",
            "--format=%(refname:short)\t%(contents:subject)",
            "refs/heads/",
            "refs/remotes/origin/",
        ],
        Some(&repo_path),
    )
    .map_err(|error| GwtError::Git(format!("for-each-ref tip subjects: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Git(format!(
            "for-each-ref tip subjects: {stderr}"
        )));
    }
    let subjects = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, subject) = line.split_once('\t')?;
            let name = name.trim();
            let subject = subject.trim();
            (!name.is_empty() && !subject.is_empty())
                .then(|| (name.to_string(), subject.to_string()))
        })
        .collect();
    Ok(subjects)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn run(cmd: &mut Command) {
        let output = cmd.output().expect("git command should run");
        assert!(
            output.status.success(),
            "git command failed: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        run(gwt_core::process::hidden_command("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(path));
        run(gwt_core::process::hidden_command("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path));
        run(gwt_core::process::hidden_command("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path));
        dir
    }

    fn create_branch(repo: &Path, name: &str) {
        run(gwt_core::process::hidden_command("git")
            .args(["branch", name])
            .current_dir(repo));
    }

    fn create_remote_tracking_ref(repo: &Path, refname: &str) {
        // Forge a remote tracking ref by writing it directly with `git
        // update-ref` so tests do not need a real remote.
        run(gwt_core::process::hidden_command("git")
            .args(["update-ref", refname, "HEAD"])
            .current_dir(repo));
    }

    fn root_tree(repo: &Path, reference: &str) -> String {
        let output = gwt_core::process::hidden_command("git")
            .args(["rev-parse", &format!("{reference}^{{tree}}")])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn canonical_root_tree_prefers_local_origin_develop_without_fetching() {
        let dir = init_repo();
        let repo = dir.path();
        std::fs::write(repo.join("source.txt"), "main\n").unwrap();
        run(gwt_core::process::hidden_command("git")
            .args(["add", "source.txt"])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "main tree"])
            .current_dir(repo));
        create_remote_tracking_ref(repo, "refs/remotes/origin/main");

        std::fs::write(repo.join("source.txt"), "develop\n").unwrap();
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-am", "develop tree"])
            .current_dir(repo));
        create_remote_tracking_ref(repo, "refs/remotes/origin/develop");

        let snapshot = resolve_canonical_root_tree(repo).unwrap().unwrap();
        assert_eq!(snapshot.reference, "origin/develop");
        assert_eq!(snapshot.root_tree_oid, root_tree(repo, "origin/develop"));
        assert_ne!(
            snapshot.root_tree_oid,
            root_tree(repo, "origin/main"),
            "precedence must select the develop tree already present locally"
        );
    }

    #[test]
    fn canonical_root_tree_returns_none_for_unborn_repository() {
        let dir = TempDir::new().unwrap();
        run(gwt_core::process::hidden_command("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(dir.path()));
        assert_eq!(resolve_canonical_root_tree(dir.path()).unwrap(), None);
    }

    #[test]
    fn canonical_root_tree_rejects_corrupt_repository_metadata() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".git"), "not a gitdir").unwrap();
        assert!(resolve_canonical_root_tree(dir.path()).is_err());
    }

    /// Issue #3629 AC-4: a workspace-home layout root is not a git work tree.
    /// The snapshot helpers must resolve the nested bare repository instead of
    /// failing — a failed snapshot forced every consumer into per-branch probe
    /// fallbacks or fail-closed dirty verdicts.
    #[test]
    fn branch_tip_committer_times_resolves_from_nested_bare_layout_root() {
        let seed = init_repo();
        create_branch(seed.path(), "work/x");
        let layout = TempDir::new().unwrap();
        let bare = layout.path().join("repo.git");
        run(gwt_core::process::hidden_command("git")
            .args([
                "clone",
                "--bare",
                seed.path().to_str().unwrap(),
                bare.to_str().unwrap(),
            ])
            .current_dir(layout.path()));

        let times = branch_tip_committer_times(layout.path()).expect("tip times from layout root");
        assert!(times.contains_key("main"));
        assert!(times.contains_key("work/x"));
    }

    #[test]
    fn branch_tip_committer_times_lists_local_and_origin_tips() {
        let dir = init_repo();
        let repo = dir.path();
        create_branch(repo, "work/x");
        create_remote_tracking_ref(repo, "refs/remotes/origin/work/y");

        let times = branch_tip_committer_times(repo).expect("tip times");
        assert!(times.contains_key("main"));
        assert!(times.contains_key("work/x"));
        assert!(times.contains_key("origin/work/y"));
        for time in times.values() {
            assert!(*time > 1_500_000_000, "plausible unix committer time");
        }
    }

    #[test]
    fn branch_tip_subjects_lists_subjects_for_local_and_origin_tips() {
        let dir = init_repo();
        let repo = dir.path();
        // A real conventional-commit subject on the default branch.
        run(gwt_core::process::hidden_command("git")
            .args([
                "commit",
                "--allow-empty",
                "-m",
                "feat(workspace): purpose-first rail",
            ])
            .current_dir(repo));
        create_branch(repo, "work/x");
        create_remote_tracking_ref(repo, "refs/remotes/origin/work/y");

        let subjects = branch_tip_subjects(repo).expect("tip subjects");
        assert_eq!(
            subjects.get("main").map(String::as_str),
            Some("feat(workspace): purpose-first rail"),
        );
        assert_eq!(
            subjects.get("work/x").map(String::as_str),
            Some("feat(workspace): purpose-first rail"),
        );
        assert!(subjects.contains_key("origin/work/y"));
    }

    #[test]
    fn list_origin_refs_with_commit_lists_refname_sha_pairs_excluding_head() {
        let dir = init_repo();
        let repo = dir.path();
        create_remote_tracking_ref(repo, "refs/remotes/origin/develop");
        create_remote_tracking_ref(repo, "refs/remotes/origin/work/x");
        // origin/HEAD symref must be skipped.
        run(gwt_core::process::hidden_command("git")
            .args([
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ])
            .current_dir(repo));

        let refs = list_origin_refs_with_commit(repo).expect("list origin refs");
        let names: Vec<&str> = refs.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"refs/remotes/origin/develop"));
        assert!(names.contains(&"refs/remotes/origin/work/x"));
        assert!(!names.contains(&"refs/remotes/origin/HEAD"));
        for (_, sha) in &refs {
            assert_eq!(sha.len(), 40, "full object sha expected: {sha}");
        }
    }

    #[test]
    fn list_existing_refs_handles_empty_input() {
        let dir = init_repo();
        let result = list_existing_refs(dir.path(), &[]).expect("empty input must succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn list_existing_refs_skips_blank_candidates() {
        let dir = init_repo();
        let result =
            list_existing_refs(dir.path(), &["", "  ", "\t"]).expect("blank input must succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn list_existing_refs_returns_only_present_refs() {
        let dir = init_repo();
        let repo = dir.path();
        create_branch(repo, "develop");
        create_remote_tracking_ref(repo, "refs/remotes/origin/develop");
        create_remote_tracking_ref(repo, "refs/remotes/origin/main");

        let result = list_existing_refs(
            repo,
            &[
                "refs/remotes/origin/develop",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
                "refs/remotes/origin/master",
            ],
        )
        .expect("for-each-ref must succeed");

        assert!(result.contains("refs/remotes/origin/develop"));
        assert!(result.contains("refs/remotes/origin/main"));
        assert!(!result.contains("refs/remotes/origin/HEAD"));
        assert!(!result.contains("refs/remotes/origin/master"));
    }

    #[test]
    fn list_existing_refs_resolves_local_and_remote_in_one_spawn() {
        let dir = init_repo();
        let repo = dir.path();
        create_branch(repo, "work/20260513-0315");
        create_remote_tracking_ref(repo, "refs/remotes/origin/work/20260513-0315");

        let result = list_existing_refs(
            repo,
            &[
                "refs/heads/work/20260513-0315",
                "refs/remotes/origin/work/20260513-0315",
                "refs/heads/feature/never-created",
                "refs/remotes/origin/feature/never-created",
            ],
        )
        .expect("for-each-ref must succeed");

        assert!(result.contains("refs/heads/work/20260513-0315"));
        assert!(result.contains("refs/remotes/origin/work/20260513-0315"));
        assert!(!result.contains("refs/heads/feature/never-created"));
        assert!(!result.contains("refs/remotes/origin/feature/never-created"));
    }
}
