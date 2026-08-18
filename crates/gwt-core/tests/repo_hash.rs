//! Phase 8: integration tests for `gwt_core::repo_hash`.
//!
//! These tests will fail to compile until `crates/gwt-core/src/repo_hash.rs`
//! exists and is exported from `lib.rs`.

use std::path::{Path, PathBuf};

use gwt_core::paths::{project_scope_hash, resolve_project_scope, ProjectScopeSource};
use gwt_core::repo_hash::{
    compute_path_hash, compute_repo_hash, normalize_origin_url, RepoHash, RepoIdentitySource,
};

#[test]
fn normalize_https_url_strips_dot_git_and_lowercases() {
    assert_eq!(
        normalize_origin_url("https://github.com/Akiojin/gwt.git"),
        "github.com/akiojin/gwt"
    );
}

#[test]
fn normalize_ssh_url_matches_https() {
    let https = normalize_origin_url("https://github.com/akiojin/gwt.git");
    let ssh = normalize_origin_url("git@github.com:akiojin/gwt.git");
    assert_eq!(https, ssh);
}

#[test]
fn normalize_ssh_with_protocol_form() {
    assert_eq!(
        normalize_origin_url("ssh://git@github.com:22/akiojin/gwt.git"),
        "github.com/akiojin/gwt"
    );
}

#[test]
fn normalize_handles_trailing_slash() {
    assert_eq!(
        normalize_origin_url("https://github.com/akiojin/gwt/"),
        "github.com/akiojin/gwt"
    );
}

#[test]
fn compute_repo_hash_returns_16_lowercase_hex_chars() {
    let h = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let hex = h.as_str();
    assert_eq!(hex.len(), 16);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hash must be lowercase hex: {hex}"
    );
}

#[test]
fn compute_repo_hash_is_deterministic() {
    let a = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let b = compute_repo_hash("https://github.com/akiojin/gwt.git");
    assert_eq!(a.as_str(), b.as_str());
}

#[test]
fn https_and_ssh_forms_yield_same_hash() {
    let a = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let b = compute_repo_hash("git@github.com:akiojin/gwt.git");
    assert_eq!(a.as_str(), b.as_str());
}

#[test]
fn different_repos_yield_different_hashes() {
    let a = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let b = compute_repo_hash("https://github.com/akiojin/other.git");
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn case_insensitive_path_yields_same_hash() {
    let a = compute_repo_hash("https://GitHub.com/Akiojin/Gwt.git");
    let b = compute_repo_hash("https://github.com/akiojin/gwt.git");
    assert_eq!(a.as_str(), b.as_str());
}

#[test]
fn repo_hash_display_equals_as_str() {
    let h: RepoHash = compute_repo_hash("https://github.com/akiojin/gwt.git");
    assert_eq!(format!("{h}"), h.as_str());
}

// ---------------------------------------------------------------------------
// Issue #3466 / SPEC-3431 T-150 (FR-072, FR-078, AS-PM-LIFECYCLE-IDENTITY):
// the Nested Bare + Worktree layout root carries no `.git` of its own, so
// `detect_repo_hash` used to return `None` and the project scope silently fell
// back to a path hash. That split the project store: the GUI (opened at the
// layout root) and agent sessions (running in linked worktrees) wrote to two
// different `~/.gwt/projects/<hash>` trees.
//
// These tests pin the settled design decision: the layout root is accepted and
// resolves to the *same* repository identity as its worktrees, via the origin
// of the bare repository nested directly beneath it.
// ---------------------------------------------------------------------------

const LAYOUT_ORIGIN: &str = "https://github.com/example/gwt.git";

fn run_git(cwd: &Path, args: &[&str]) {
    let mut cmd = gwt_core::process::hidden_command("git");
    cmd.args(args).current_dir(cwd);
    gwt_core::process::scrub_git_env(&mut cmd);
    let output = cmd.output().expect("git command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Build a Nested Bare + Worktree layout under `root`:
///
/// ```text
/// root/                 <- layout root, no `.git`
///   gwt.git/            <- bare repository holding `origin`
///   work/develop/       <- linked worktree
/// ```
///
/// Returns `(bare_repo, linked_worktree)`.
fn make_layout_root(root: &Path, origin: &str) -> (PathBuf, PathBuf) {
    std::fs::create_dir_all(root).expect("layout root");
    let bare = root.join("gwt.git");
    let bootstrap = root.join(".bootstrap");
    let worktree = root.join("work").join("develop");
    std::fs::create_dir_all(worktree.parent().expect("work dir")).expect("work dir");

    run_git(root, &["init", "--bare", bare.to_str().expect("bare path")]);
    run_git(&bare, &["remote", "add", "origin", origin]);
    run_git(
        root,
        &["clone", bare.to_str().expect("bare path"), ".bootstrap"],
    );
    run_git(&bootstrap, &["checkout", "-b", "develop"]);
    run_git(
        &bootstrap,
        &[
            "-c",
            "user.name=gwt-test",
            "-c",
            "user.email=gwt-test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    run_git(&bootstrap, &["push", "origin", "develop"]);
    std::fs::remove_dir_all(&bootstrap).expect("remove bootstrap");
    run_git(
        &bare,
        &[
            "worktree",
            "add",
            worktree.to_str().expect("worktree path"),
            "develop",
        ],
    );

    (bare, worktree)
}

/// AC-1 / AC-2: opening the layout root and opening a linked worktree must
/// land in one project store.
#[test]
fn layout_root_and_linked_worktree_share_one_project_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (_bare, worktree) = make_layout_root(&root, LAYOUT_ORIGIN);

    let expected = compute_repo_hash(LAYOUT_ORIGIN);
    assert_eq!(
        project_scope_hash(&root).as_str(),
        expected.as_str(),
        "layout root must resolve to the origin identity, not a path hash"
    );
    assert_eq!(
        project_scope_hash(&worktree).as_str(),
        expected.as_str(),
        "linked worktree must resolve to the origin identity"
    );
}

/// AC-5: a detached-HEAD worktree (how the PM lane materializes workspaces)
/// resolves to the same store as the layout root.
#[test]
fn detached_worktree_shares_the_layout_root_project_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (bare, _worktree) = make_layout_root(&root, LAYOUT_ORIGIN);

    let detached = root.join("work").join("pm-detached");
    run_git(
        &bare,
        &[
            "worktree",
            "add",
            "--detach",
            detached.to_str().expect("detached path"),
            "develop",
        ],
    );

    assert_eq!(
        project_scope_hash(&detached).as_str(),
        project_scope_hash(&root).as_str(),
        "detached PM worktree must share the layout root store"
    );
}

/// Issue #3606 AC-5: the PM lane materializes its worktree *inside* a project
/// store directory (`~/.gwt/projects/<hash>/pm/worktree`), so its path is
/// keyed by whichever hash was canonical when it was created. Resolution must
/// still follow the repository's origin: keying on the enclosing directory
/// would pin the PM to a store that an upgrade can move out from under it,
/// which is exactly how #3606 stranded the PM's whole observation surface.
#[test]
fn pm_worktree_inside_a_project_store_resolves_to_the_repository_identity() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (bare, _worktree) = make_layout_root(&root, LAYOUT_ORIGIN);

    // The directory name is the *legacy* path hash, the way a store created
    // before the identity convergence would have been named.
    let legacy_hash = compute_path_hash(&root);
    let pm_worktree = gwt_core::paths::gwt_projects_dir()
        .join(legacy_hash.as_str())
        .join("pm")
        .join("worktree");
    std::fs::create_dir_all(pm_worktree.parent().expect("pm dir")).expect("pm dir");
    run_git(
        &bare,
        &[
            "worktree",
            "add",
            "--detach",
            pm_worktree.to_str().expect("pm worktree path"),
            "develop",
        ],
    );

    let expected = compute_repo_hash(LAYOUT_ORIGIN);
    assert_eq!(
        project_scope_hash(&pm_worktree).as_str(),
        expected.as_str(),
        "a PM worktree must resolve by origin, not by the store directory it sits in"
    );
    assert_ne!(
        project_scope_hash(&pm_worktree).as_str(),
        legacy_hash.as_str(),
        "the enclosing store's hash must not capture the identity"
    );
    assert_eq!(
        project_scope_hash(&pm_worktree).as_str(),
        project_scope_hash(&root).as_str(),
        "PM and the layout root must observe one store"
    );
}

/// Issue #3606 AC-4: which stores already exist on disk must never influence
/// where a project root resolves. #3606 was an upgrade silently changing the
/// answer for an unchanged project root; pinning resolution to the repository
/// alone is what makes the answer stable across restarts and versions, and
/// leaves the stranded store a migration problem rather than an ambiguity.
#[test]
fn existing_stores_never_change_where_a_project_root_resolves() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (_bare, worktree) = make_layout_root(&root, LAYOUT_ORIGIN);
    let expected = compute_repo_hash(LAYOUT_ORIGIN);

    let before_root = project_scope_hash(&root);
    let before_worktree = project_scope_hash(&worktree);
    assert_eq!(before_root.as_str(), expected.as_str());

    // Materialize the legacy path-hash store a pre-#3466 build would have
    // written, and the identity store beside it.
    for hash in [compute_path_hash(&root), expected.clone()] {
        std::fs::create_dir_all(
            gwt_core::paths::gwt_projects_dir()
                .join(hash.as_str())
                .join("project-state"),
        )
        .expect("materialize store");
    }

    assert_eq!(
        project_scope_hash(&root).as_str(),
        before_root.as_str(),
        "an existing legacy store must not pull the project root back to it"
    );
    assert_eq!(
        project_scope_hash(&worktree).as_str(),
        before_worktree.as_str(),
        "nor may it change where a linked worktree resolves"
    );
    assert_eq!(
        project_scope_hash(&root).as_str(),
        project_scope_hash(&worktree).as_str(),
        "both entry points stay on one store"
    );
}

/// AC-3: the resolution source is observable, so a path-hash fallback can be
/// diagnosed instead of silently splitting the store.
#[test]
fn project_scope_reports_how_the_identity_was_resolved() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (bare, worktree) = make_layout_root(&root, LAYOUT_ORIGIN);

    match resolve_project_scope(&root).source {
        ProjectScopeSource::Repository(RepoIdentitySource::NestedBareRepository(reported)) => {
            assert!(
                reported.ends_with("gwt.git"),
                "layout root resolution must name the nested bare repository it used, got {}",
                reported.display()
            );
            assert!(bare.exists(), "fixture bare repository must exist");
        }
        other => panic!("layout root must resolve through a nested bare repository, got {other:?}"),
    }

    assert_eq!(
        resolve_project_scope(&worktree).source,
        ProjectScopeSource::Repository(RepoIdentitySource::Origin),
        "a worktree resolves through its own origin"
    );

    let plain = tmp.path().join("not-a-repo");
    std::fs::create_dir_all(&plain).expect("plain dir");
    assert_eq!(
        resolve_project_scope(&plain).source,
        ProjectScopeSource::PathFallback,
        "non-repository paths must report the fallback instead of failing silently"
    );
}

/// AC-9 negative: a directory that merely sits next to a repository must not
/// be absorbed into that repository's store.
#[test]
fn unrelated_directory_keeps_its_own_path_scoped_store() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    make_layout_root(&root, LAYOUT_ORIGIN);

    let unrelated = tmp.path().join("unrelated");
    std::fs::create_dir_all(&unrelated).expect("unrelated dir");

    assert_eq!(
        project_scope_hash(&unrelated).as_str(),
        compute_path_hash(&unrelated).as_str(),
        "a sibling directory without its own repository stays path scoped"
    );
    assert_ne!(
        project_scope_hash(&unrelated).as_str(),
        project_scope_hash(&root).as_str(),
        "an unrelated directory must never join another repository's store"
    );
}

/// AC-9 negative: only *direct* children are considered, so a deep checkout
/// cannot hijack the identity of an enclosing directory.
#[test]
fn nested_bare_lookup_does_not_descend_past_direct_children() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let outer = tmp.path().join("outer");
    let inner = outer.join("deep").join("workbench");
    make_layout_root(&inner, LAYOUT_ORIGIN);

    assert_eq!(
        project_scope_hash(&outer).as_str(),
        compute_path_hash(&outer).as_str(),
        "a grandparent directory must not inherit a nested repository's identity"
    );
}

/// AC-9: multiple bare repositories for the same origin still describe one
/// repository identity.
#[test]
fn multiple_same_origin_child_bare_repositories_share_identity() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (_bare, _worktree) = make_layout_root(&root, LAYOUT_ORIGIN);

    let second = root.join("zz-same-origin.git");
    run_git(&root, &["init", "--bare", second.to_str().expect("second")]);
    run_git(
        &second,
        &["remote", "add", "origin", "git@github.com:example/gwt.git"],
    );

    assert_eq!(
        project_scope_hash(&root).as_str(),
        compute_repo_hash(LAYOUT_ORIGIN).as_str(),
        "duplicate bare repositories for one origin must share its identity"
    );
}

/// AC-9 negative: distinct origins under one layout root are ambiguous. The
/// resolver must not choose a repository based on candidate path ordering.
#[test]
fn multiple_distinct_origin_child_bare_repositories_fail_closed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (_bare, _worktree) = make_layout_root(&root, LAYOUT_ORIGIN);

    let second = root.join("aa-other.git");
    run_git(&root, &["init", "--bare", second.to_str().expect("second")]);
    run_git(
        &second,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/other.git",
        ],
    );

    let scope = resolve_project_scope(&root);
    assert_eq!(
        scope.hash.as_str(),
        compute_path_hash(&root).as_str(),
        "an ambiguous layout root must not join either repository store"
    );
    match scope.source {
        ProjectScopeSource::AmbiguousNestedBareRepositories(candidates) => {
            assert_eq!(candidates.len(), 2, "both candidates must be diagnosed");
            let origins: Vec<_> = candidates
                .iter()
                .map(|candidate| candidate.normalized_origin.as_str())
                .collect();
            assert_eq!(
                origins,
                vec!["github.com/example/other", "github.com/example/gwt"],
                "diagnostics must identify every distinct candidate origin"
            );
            assert!(
                candidates[0].path.ends_with("aa-other.git")
                    && candidates[1].path.ends_with("gwt.git"),
                "diagnostics must identify every candidate path: {candidates:?}"
            );
        }
        other => panic!("distinct origins must be reported as ambiguous, got {other:?}"),
    }

    let renamed = root.join("zz-other.git");
    std::fs::rename(&second, &renamed).expect("rename second bare repository");
    let renamed_scope = resolve_project_scope(&root);
    assert_eq!(
        renamed_scope.hash.as_str(),
        compute_path_hash(&root).as_str(),
        "renaming a candidate must not make an ambiguous layout select a repository"
    );
    match renamed_scope.source {
        ProjectScopeSource::AmbiguousNestedBareRepositories(candidates) => {
            let mut origins: Vec<_> = candidates
                .iter()
                .map(|candidate| candidate.normalized_origin.as_str())
                .collect();
            origins.sort_unstable();
            assert_eq!(
                origins,
                vec!["github.com/example/gwt", "github.com/example/other"],
                "renaming only changes discovery order, not ambiguity"
            );
        }
        other => panic!("renamed distinct origins must remain ambiguous, got {other:?}"),
    }
}

/// A direct child with a missing or empty origin cannot make a unique valid
/// repository identity ambiguous.
#[test]
fn malformed_or_origin_missing_child_bare_repository_is_ignored() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workbench");
    let (_bare, _worktree) = make_layout_root(&root, LAYOUT_ORIGIN);

    let missing = root.join("aa-missing-origin.git");
    run_git(
        &root,
        &["init", "--bare", missing.to_str().expect("missing origin")],
    );
    let malformed = root.join("zz-empty-origin.git");
    run_git(
        &root,
        &["init", "--bare", malformed.to_str().expect("empty origin")],
    );
    run_git(&malformed, &["config", "remote.origin.url", "   "]);

    assert_eq!(
        project_scope_hash(&root).as_str(),
        compute_repo_hash(LAYOUT_ORIGIN).as_str(),
        "a candidate without origin must not hide the unique valid identity"
    );
}
