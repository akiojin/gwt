use std::path::{Path, PathBuf};

use super::compare;

const HEAD_BRANCH: &str = "work/issue-4979";
const BASE_BRANCH: &str = "develop";

fn git(repo: &Path, args: &[&str]) -> String {
    let output = gwt_core::process::hidden_command("git")
        .args(["-c", "core.hooksPath=", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repo)
        .output()
        .expect("spawn fixture git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 fixture git output")
        .trim()
        .to_string()
}

struct RemoteFixture {
    _root: tempfile::TempDir,
    verified_repo: PathBuf,
    other_clone: PathBuf,
    verified_head: String,
}

impl RemoteFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("remote-head fixture");
        let remote = root.path().join("remote.git");
        let verified_repo = root.path().join("verified");
        let other_clone = root.path().join("other");
        git(
            root.path(),
            &[
                "init",
                "--bare",
                "--quiet",
                "--initial-branch=develop",
                remote.to_str().unwrap(),
            ],
        );
        git(
            root.path(),
            &[
                "clone",
                "--quiet",
                remote.to_str().unwrap(),
                verified_repo.to_str().unwrap(),
            ],
        );
        Self::configure(&verified_repo);
        std::fs::write(verified_repo.join("src.txt"), "verified source\n").unwrap();
        git(&verified_repo, &["add", "src.txt"]);
        git(
            &verified_repo,
            &["commit", "--quiet", "-m", "feat: seed base"],
        );
        git(
            &verified_repo,
            &["push", "--quiet", "-u", "origin", BASE_BRANCH],
        );
        git(&verified_repo, &["checkout", "--quiet", "-b", HEAD_BRANCH]);
        std::fs::write(verified_repo.join("feature.txt"), "verified feature\n").unwrap();
        git(&verified_repo, &["add", "feature.txt"]);
        git(
            &verified_repo,
            &["commit", "--quiet", "-m", "feat: verified work"],
        );
        git(
            &verified_repo,
            &["push", "--quiet", "-u", "origin", HEAD_BRANCH],
        );
        let verified_head = git(&verified_repo, &["rev-parse", "HEAD"]);
        git(
            root.path(),
            &[
                "clone",
                "--quiet",
                remote.to_str().unwrap(),
                other_clone.to_str().unwrap(),
            ],
        );
        Self::configure(&other_clone);
        git(&other_clone, &["checkout", "--quiet", HEAD_BRANCH]);
        Self {
            _root: root,
            verified_repo,
            other_clone,
            verified_head,
        }
    }

    fn configure(repo: &Path) {
        git(repo, &["config", "user.name", "Remote Head Test"]);
        git(repo, &["config", "user.email", "remote-head@example.com"]);
    }

    fn commit_other(&self, path: &str, content: &str, subject: &str) -> String {
        let destination = self.other_clone.join(path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, content).unwrap();
        git(&self.other_clone, &["add", "--", path]);
        git(&self.other_clone, &["commit", "--quiet", "-m", subject]);
        git(&self.other_clone, &["rev-parse", "HEAD"])
    }

    fn push_head(&self) {
        git(
            &self.other_clone,
            &["push", "--quiet", "origin", HEAD_BRANCH],
        );
    }

    fn sync_changed_base(&self) -> String {
        git(&self.other_clone, &["checkout", "--quiet", BASE_BRANCH]);
        let base_head = self.commit_other("base.txt", "new base source\n", "feat: base update");
        git(
            &self.other_clone,
            &["push", "--quiet", "origin", BASE_BRANCH],
        );
        git(&self.other_clone, &["checkout", "--quiet", HEAD_BRANCH]);
        git(
            &self.other_clone,
            &[
                "merge",
                "--quiet",
                "--no-ff",
                "-m",
                "chore: sync base",
                BASE_BRANCH,
            ],
        );
        self.push_head();
        base_head
    }

    fn check(&self) -> super::HeadCheck {
        compare(
            &self.verified_repo,
            "origin",
            HEAD_BRANCH,
            BASE_BRANCH,
            &self.verified_head,
        )
        .expect("compare the remote snapshot")
    }
}

#[test]
fn remote_only_product_commit_is_rejected_even_after_revert() {
    let fixture = RemoteFixture::new();
    assert!(
        fixture.check().allowed(),
        "the exact verified HEAD is allowed"
    );
    let product_commit =
        fixture.commit_other("src.txt", "unverified source\n", "fix: remote product");
    fixture.push_head();

    let check = fixture.check();
    assert!(
        !check.allowed(),
        "remote-only product must require verification"
    );
    assert_eq!(check.verified_head, fixture.verified_head);
    assert_eq!(check.remote_head, product_commit);
    assert!(check
        .product_commits
        .iter()
        .any(|commit| commit.contains(&product_commit)));
    assert!(check.product_files.iter().any(|path| path == "src.txt"));
    let subdir = fixture.verified_repo.join("nested");
    std::fs::create_dir(&subdir).unwrap();
    let nested = compare(
        &subdir,
        "origin",
        HEAD_BRANCH,
        BASE_BRANCH,
        &fixture.verified_head,
    )
    .unwrap();
    assert!(
        !nested.allowed(),
        "the caller's cwd must not hide product changes"
    );
    assert_eq!(nested.product_commits, check.product_commits);
    assert_eq!(nested.product_files, check.product_files);
    assert_eq!(
        git(&fixture.verified_repo, &["rev-parse", "HEAD"]),
        fixture.verified_head
    );

    git(
        &fixture.other_clone,
        &["revert", "--no-edit", &product_commit],
    );
    fixture.push_head();
    let reverted = fixture.check();
    assert!(
        !reverted.allowed(),
        "a revert must not hide unverified product history"
    );
    assert!(reverted
        .product_commits
        .iter()
        .any(|commit| commit.contains(&product_commit)));
    assert!(reverted.product_files.iter().any(|path| path == "src.txt"));
}

#[test]
fn remote_only_gwt_bookkeeping_is_allowed() {
    let fixture = RemoteFixture::new();
    let remote_head =
        fixture.commit_other(".gwt/work/status.json", "{}\n", "chore(work): bookkeeping");
    fixture.push_head();

    let check = fixture.check();
    assert!(
        check.allowed(),
        "bookkeeping alone must not require product verification"
    );
    assert_eq!(check.remote_head, remote_head);
    assert!(check.product_commits.is_empty());
    assert!(check.product_files.is_empty());
    assert_eq!(
        git(&fixture.verified_repo, &["rev-parse", "HEAD"]),
        fixture.verified_head
    );
}

#[test]
fn source_changing_base_sync_alone_is_allowed() {
    let fixture = RemoteFixture::new();
    let base_head = fixture.sync_changed_base();

    let check = fixture.check();
    assert!(
        check.allowed(),
        "source already in the PR base is a permitted base sync"
    );
    assert_eq!(check.base_head, base_head);
    assert!(check.product_commits.is_empty());
    assert!(check.product_files.is_empty());
    assert_eq!(
        git(&fixture.verified_repo, &["rev-parse", "HEAD"]),
        fixture.verified_head
    );
}

#[test]
fn base_sync_does_not_hide_a_remote_only_product_commit() {
    let fixture = RemoteFixture::new();
    let product_commit =
        fixture.commit_other("src.txt", "unverified source\n", "fix: remote product");
    fixture.sync_changed_base();

    let check = fixture.check();
    assert!(
        !check.allowed(),
        "a base merge must not exempt an unverified product parent"
    );
    assert!(check
        .product_commits
        .iter()
        .any(|commit| commit.contains(&product_commit)));
    assert!(check.product_files.iter().any(|path| path == "src.txt"));
    assert!(!check.product_files.iter().any(|path| path == "base.txt"));
}

#[test]
fn later_merge_cannot_hide_an_unverified_product_merge_resolution() {
    let fixture = RemoteFixture::new();
    let side_branch = "bookkeeping-side";
    git(
        &fixture.other_clone,
        &["checkout", "--quiet", "-b", side_branch],
    );
    fixture.commit_other(
        ".gwt/side.txt",
        "first side update\n",
        "chore(work): side update",
    );
    git(&fixture.other_clone, &["checkout", "--quiet", HEAD_BRANCH]);
    fixture.commit_other(".gwt/head.txt", "head update\n", "chore(work): head update");
    git(
        &fixture.other_clone,
        &["merge", "--quiet", "--no-ff", "--no-commit", side_branch],
    );
    let product_merge = fixture.commit_other(
        "src.txt",
        "unverified merge resolution\n",
        "fix: product merge resolution",
    );

    git(&fixture.other_clone, &["checkout", "--quiet", side_branch]);
    fixture.commit_other(
        ".gwt/side.txt",
        "second side update\n",
        "chore(work): next side update",
    );
    git(&fixture.other_clone, &["checkout", "--quiet", HEAD_BRANCH]);
    git(
        &fixture.other_clone,
        &["merge", "--quiet", "--no-ff", "--no-commit", side_branch],
    );
    let original_source = std::fs::read_to_string(fixture.verified_repo.join("src.txt")).unwrap();
    fixture.commit_other(
        "src.txt",
        &original_source,
        "fix: restore source during merge",
    );
    fixture.push_head();
    git(
        &fixture.other_clone,
        &[
            "diff",
            "--quiet",
            &fixture.verified_head,
            "HEAD",
            "--",
            "src.txt",
        ],
    );

    let check = fixture.check();
    assert!(
        !check.allowed(),
        "restoring source must not hide a product merge resolution"
    );
    assert!(check
        .product_commits
        .iter()
        .any(|commit| commit.contains(&product_merge)));
    assert!(check.product_files.iter().any(|path| path == "src.txt"));
}

#[test]
fn divergent_or_unavailable_remote_cannot_authorize_a_handoff() {
    let fixture = RemoteFixture::new();
    git(&fixture.other_clone, &["checkout", "--quiet", BASE_BRANCH]);
    fixture.commit_other("other.txt", "divergent source\n", "fix: unrelated branch");
    git(
        &fixture.other_clone,
        &[
            "push",
            "--quiet",
            "--force",
            "origin",
            &format!("HEAD:refs/heads/{HEAD_BRANCH}"),
        ],
    );
    if let Ok(check) = compare(
        &fixture.verified_repo,
        "origin",
        HEAD_BRANCH,
        BASE_BRANCH,
        &fixture.verified_head,
    ) {
        assert!(
            !check.allowed(),
            "divergent history cannot prove the verified snapshot"
        );
    }
    assert!(compare(
        &fixture.verified_repo,
        "missing-remote",
        HEAD_BRANCH,
        BASE_BRANCH,
        &fixture.verified_head,
    )
    .is_err());
}
