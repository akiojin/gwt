//! SPEC-2359 W-16 (FR-387): checkout-free blob access for the cross-machine
//! work events intake.
//!
//! Spawn budget (plan §Architecture Decisions 4): one `cat-file
//! --batch-check` resolves the `events.jsonl` blob oid for ANY number of
//! refs (object list rides stdin), then one `cat-file --batch` reads every
//! not-yet-ingested unique blob.

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

/// Read multiple blobs by oid in one `git cat-file --batch` process.
///
/// The returned contents preserve `oids` input order, including duplicate
/// oids. Callers that fan one blob out to many refs may deduplicate the input
/// first to avoid repeating large payloads on the batch protocol.
pub fn read_blobs_batch(repo_path: &Path, oids: &[String]) -> Result<Vec<String>> {
    if oids.is_empty() {
        return Ok(Vec::new());
    }
    let stdin = oids.join("\n") + "\n";
    let output = gwt_core::process::run_git_logged_with_stdin(
        &["cat-file", "--batch"],
        Some(repo_path),
        stdin.as_bytes(),
    )
    .map_err(|error| GwtError::Git(format!("cat-file --batch: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GwtError::Git(format!("cat-file --batch: {stderr}")));
    }
    parse_batch_blob_contents(&output.stdout, oids.len())
}

fn parse_batch_blob_contents(output: &[u8], expected: usize) -> Result<Vec<String>> {
    let mut cursor = 0usize;
    let mut contents = Vec::with_capacity(expected);
    for index in 0..expected {
        let header_end = output[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| cursor + offset)
            .ok_or_else(|| {
                GwtError::Git(format!(
                    "cat-file --batch: missing header terminator for blob {index}"
                ))
            })?;
        let header = std::str::from_utf8(&output[cursor..header_end]).map_err(|error| {
            GwtError::Git(format!(
                "cat-file --batch: invalid header for blob {index}: {error}"
            ))
        })?;
        let mut fields = header.split_whitespace();
        let object = fields.next();
        let kind = fields.next();
        if let (Some(object), Some("missing")) = (object, kind) {
            return Err(GwtError::Git(format!(
                "cat-file --batch: missing object for blob {index}: {object}"
            )));
        }
        let size = fields.next().and_then(|value| value.parse::<usize>().ok());
        let (Some("blob"), Some(size)) = (kind, size) else {
            return Err(GwtError::Git(format!(
                "cat-file --batch: unexpected header for blob {index}: {header}"
            )));
        };
        let content_start = header_end + 1;
        let content_end = content_start.checked_add(size).ok_or_else(|| {
            GwtError::Git(format!("cat-file --batch: size overflow for blob {index}"))
        })?;
        if content_end >= output.len() || output[content_end] != b'\n' {
            return Err(GwtError::Git(format!(
                "cat-file --batch: truncated content for blob {index}"
            )));
        }
        contents.push(String::from_utf8_lossy(&output[content_start..content_end]).into_owned());
        cursor = content_end + 1;
    }
    if cursor != output.len() {
        return Err(GwtError::Git(format!(
            "cat-file --batch: unexpected trailing output ({} bytes)",
            output.len() - cursor
        )));
    }
    Ok(contents)
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

    #[test]
    fn batch_reads_multiple_and_duplicate_blobs_in_input_order() {
        let dir = init_repo();
        let repo = dir.path();
        std::fs::write(repo.join("first.txt"), "first\nline\n").expect("first blob");
        std::fs::write(repo.join("second.txt"), "second without newline").expect("second blob");
        let first = gwt_core::process::hidden_command("git")
            .args(["hash-object", "-w", "first.txt"])
            .current_dir(repo)
            .output()
            .expect("hash first");
        let second = gwt_core::process::hidden_command("git")
            .args(["hash-object", "-w", "second.txt"])
            .current_dir(repo)
            .output()
            .expect("hash second");
        let first = String::from_utf8_lossy(&first.stdout).trim().to_string();
        let second = String::from_utf8_lossy(&second.stdout).trim().to_string();

        let contents =
            read_blobs_batch(repo, &[first.clone(), second, first]).expect("batch blob read");

        assert_eq!(
            contents,
            vec![
                "first\nline\n".to_string(),
                "second without newline".to_string(),
                "first\nline\n".to_string(),
            ]
        );
    }

    #[test]
    fn batch_reads_no_blobs_without_spawning() {
        let dir = init_repo();
        assert!(read_blobs_batch(dir.path(), &[]).unwrap().is_empty());
    }

    #[test]
    fn batch_parser_reports_missing_object_with_input_index() {
        let error = parse_batch_blob_contents(b"missing-oid missing\n", 1)
            .expect_err("missing object must fail the batch");

        assert!(
            error
                .to_string()
                .contains("cat-file --batch: missing object for blob 0: missing-oid"),
            "missing object diagnosis must identify the input slot and spec: {error}"
        );
    }

    #[test]
    fn batch_parser_rejects_truncated_payload() {
        let error = parse_batch_blob_contents(b"blob-oid blob 5\nabc\n", 1)
            .expect_err("short payload must fail the batch");

        assert!(
            error
                .to_string()
                .contains("cat-file --batch: truncated content for blob 0"),
            "truncated payload must not be accepted: {error}"
        );
    }

    #[test]
    fn batch_parser_rejects_trailing_bytes() {
        let error = parse_batch_blob_contents(b"blob-oid blob 3\nabc\nextra", 1)
            .expect_err("trailing bytes must fail the batch");

        assert!(
            error
                .to_string()
                .contains("cat-file --batch: unexpected trailing output (5 bytes)"),
            "trailing output must not be silently discarded: {error}"
        );
    }

    #[test]
    fn batch_parser_rejects_payload_size_overflow() {
        let output = format!("blob-oid blob {}\n", usize::MAX);
        let error = parse_batch_blob_contents(output.as_bytes(), 1)
            .expect_err("overflowing payload boundary must fail the batch");

        assert!(
            error
                .to_string()
                .contains("cat-file --batch: size overflow for blob 0"),
            "overflow must be distinguished from ordinary truncation: {error}"
        );
    }
}
