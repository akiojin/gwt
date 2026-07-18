//! SPEC-2359 W-16 (FR-387): checkout-free blob access for the cross-machine
//! work events intake.
//!
//! Spawn budget (plan §Architecture Decisions 4): one `cat-file
//! --batch-check` resolves the `events.jsonl` blob oid for ANY number of
//! refs (object list rides stdin), and only blobs that have not been
//! ingested yet pay an extra `cat-file blob` read.

use std::path::Path;

use gwt_core::{GwtError, Result};

/// Resolve the blob oid of `path_in_tree` for each commit sha in `commits`,
/// in ONE `git cat-file --batch-check` spawn. Returns one entry per input
/// commit, `None` when the commit's tree has no such path.
pub fn events_blob_oids_batch(
    repo_path: &Path,
    commits: &[String],
    path_in_tree: &str,
) -> Result<Vec<Option<String>>> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let stdin: String = commits
        .iter()
        .map(|sha| format!("{sha}:{path_in_tree}\n"))
        .collect();
    let output = gwt_core::process::run_git_logged_with_stdin(
        &["cat-file", "--batch-check"],
        Some(repo_path),
        stdin.as_bytes(),
    )
    .map_err(|error| GwtError::Git(format!("cat-file --batch-check: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Git(format!("cat-file --batch-check: {stderr}")));
    }
    // One output line per input line, in order:
    //   `<oid> <type> <size>` for resolvable objects,
    //   `<spec> missing` (or `... ambiguous`) otherwise.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let oids: Vec<Option<String>> = stdout
        .lines()
        .map(|line| {
            let mut parts = line.split_whitespace();
            let first = parts.next()?.to_string();
            let second = parts.next()?;
            (second == "blob").then_some(first)
        })
        .collect();
    if oids.len() != commits.len() {
        return Err(GwtError::Git(format!(
            "cat-file --batch-check: expected {} lines, got {}",
            commits.len(),
            oids.len()
        )));
    }
    Ok(oids)
}

/// Read a blob's full content by oid — no checkout, no worktree access.
pub fn read_blob(repo_path: &Path, oid: &str) -> Result<String> {
    let output = gwt_core::process::run_git_logged(&["cat-file", "blob", oid], Some(repo_path))
        .map_err(|error| GwtError::Git(format!("cat-file blob: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Git(format!("cat-file blob {oid}: {stderr}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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

    fn head_sha(repo: &std::path::Path) -> String {
        let output = gwt_core::process::hidden_command("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .expect("rev-parse");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn batch_resolves_events_blob_oids_and_reads_without_checkout() {
        let dir = init_repo();
        let repo = dir.path();

        // Commit with .gwt/work/events.jsonl on a side branch.
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/with-events"])
            .current_dir(repo));
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("mk .gwt/work");
        std::fs::write(repo.join(".gwt/work/events.jsonl"), "{\"id\":\"evt-1\"}\n")
            .expect("write events");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events.jsonl"])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "events"])
            .current_dir(repo));
        let with_events = head_sha(repo);

        // Back to main (no events file) and drop the working copy so a
        // successful read proves checkout-free access.
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(repo));
        let without_events = head_sha(repo);
        assert!(
            !repo.join(".gwt/work/events.jsonl").exists(),
            "fixture: main checkout must not carry the events file"
        );

        let oids = events_blob_oids_batch(
            repo,
            &[with_events, without_events],
            ".gwt/work/events.jsonl",
        )
        .expect("batch-check");
        assert_eq!(oids.len(), 2);
        let blob_oid = oids[0].as_deref().expect("events blob resolves");
        assert!(oids[1].is_none(), "ref without events.jsonl yields None");

        let content = read_blob(repo, blob_oid).expect("read blob");
        assert_eq!(content, "{\"id\":\"evt-1\"}\n");
    }

    #[test]
    fn batch_with_no_commits_spawns_nothing_and_returns_empty() {
        let dir = init_repo();
        let oids =
            events_blob_oids_batch(dir.path(), &[], ".gwt/work/events.jsonl").expect("empty");
        assert!(oids.is_empty());
    }
}
