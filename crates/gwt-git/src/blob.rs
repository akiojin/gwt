//! SPEC-2359 W-16 (FR-387): checkout-free blob access for the cross-machine
//! work events intake.
//!
//! The legacy batch API remains intact. Canonical event shards add one
//! checkout-free `ls-tree -r -z` per unique commit; shared commits and blob
//! oids are deduplicated before the single `cat-file --batch` content read.

use std::{collections::HashMap, path::Path};

use gwt_core::{GwtError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeBlobEntry {
    pub path: String,
    pub oid: String,
    pub mode: String,
}

/// Enumerate every blob below `path_in_tree` for each commit without checking
/// it out. Duplicate commits are listed once and fanned back out in input
/// order; blob contents remain the responsibility of [`read_blobs_batch`].
pub fn tree_blob_entries_batch(
    repo_path: &Path,
    commits: &[String],
    path_in_tree: &str,
) -> Result<Vec<Vec<TreeBlobEntry>>> {
    let mut entries_by_commit = HashMap::<String, Vec<TreeBlobEntry>>::new();
    for commit in commits {
        if entries_by_commit.contains_key(commit) {
            continue;
        }
        let output = gwt_core::process::run_git_logged(
            &["ls-tree", "-r", "-z", commit, "--", path_in_tree],
            Some(repo_path),
        )
        .map_err(|error| GwtError::Git(format!("ls-tree {commit}: {error}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GwtError::Git(format!("ls-tree {commit}: {stderr}")));
        }
        let entries = parse_tree_blob_entries(&output.stdout)?;
        entries_by_commit.insert(commit.clone(), entries);
    }
    commits
        .iter()
        .map(|commit| {
            entries_by_commit.get(commit).cloned().ok_or_else(|| {
                GwtError::Git(format!("ls-tree {commit}: missing enumerated commit"))
            })
        })
        .collect()
}

fn parse_tree_blob_entries(output: &[u8]) -> Result<Vec<TreeBlobEntry>> {
    output
        .split(|byte| *byte == b'\0')
        .filter(|record| !record.is_empty())
        .map(|record| {
            let separator = record
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| GwtError::Git("ls-tree: missing path separator".to_string()))?;
            let (metadata, path_with_separator) = record.split_at(separator);
            let path = &path_with_separator[1..];
            let metadata = std::str::from_utf8(metadata)
                .map_err(|error| GwtError::Git(format!("ls-tree: invalid metadata: {error}")))?;
            let mut fields = metadata.split_whitespace();
            let mode = fields.next();
            let kind = fields.next();
            let oid = fields.next();
            if !matches!(mode, Some("100644" | "100755"))
                || kind != Some("blob")
                || oid.is_none()
                || fields.next().is_some()
            {
                return Err(GwtError::Git(format!(
                    "ls-tree: unexpected entry metadata: {metadata}"
                )));
            }
            let path = std::str::from_utf8(path)
                .map_err(|error| GwtError::Git(format!("ls-tree: invalid path: {error}")))?;
            Ok(TreeBlobEntry {
                path: path.to_string(),
                oid: oid.unwrap().to_string(),
                mode: mode.unwrap().to_string(),
            })
        })
        .collect()
}

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
    read_blob_bytes_batch(repo_path, oids).map(|contents| {
        contents
            .into_iter()
            .map(|content| String::from_utf8_lossy(&content).into_owned())
            .collect()
    })
}

/// Byte-preserving variant of [`read_blobs_batch`] for callers that must
/// validate UTF-8 and exact record framing themselves.
pub fn read_blob_bytes_batch(repo_path: &Path, oids: &[String]) -> Result<Vec<Vec<u8>>> {
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

fn parse_batch_blob_contents(output: &[u8], expected: usize) -> Result<Vec<Vec<u8>>> {
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
        contents.push(output[content_start..content_end].to_vec());
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
    fn batch_lists_tree_blobs_below_event_store_without_checkout() {
        let dir = init_repo();
        let repo = dir.path();
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/sharded-events"])
            .current_dir(repo));
        std::fs::create_dir_all(repo.join(".gwt/work/events")).expect("event store");
        std::fs::write(repo.join(".gwt/work/events.jsonl"), "legacy\n").expect("legacy");
        std::fs::write(repo.join(".gwt/work/events/aaaaaaaa.jsonl"), "first\n")
            .expect("first shard");
        std::fs::write(repo.join(".gwt/work/events/bbbbbbbb.jsonl"), "second\n")
            .expect("second shard");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work"])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "sharded events"])
            .current_dir(repo));
        let with_events = head_sha(repo);
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(repo));
        let without_events = head_sha(repo);

        let entries =
            tree_blob_entries_batch(repo, &[with_events, without_events], ".gwt/work/events")
                .expect("tree listing");

        assert_eq!(entries.len(), 2);
        let paths = entries[0]
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                ".gwt/work/events/aaaaaaaa.jsonl",
                ".gwt/work/events/bbbbbbbb.jsonl",
            ]
        );
        assert!(entries[0].iter().all(|entry| entry.oid.len() == 40));
        assert!(entries[0].iter().all(|entry| entry.mode == "100644"));
        assert!(entries[1].is_empty());
    }

    #[test]
    fn tree_blob_parser_rejects_symlink_mode_even_when_object_kind_is_blob() {
        let output = b"120000 blob 0123456789012345678901234567890123456789\t.gwt/work/events/aaaaaaaa.jsonl\0";

        let error = parse_tree_blob_entries(output)
            .expect_err("a Git symlink blob must not be exposed as a regular event shard");

        assert!(
            error.to_string().contains("unexpected entry metadata"),
            "mode rejection should retain the ls-tree record context: {error}"
        );
    }

    #[test]
    fn tree_blob_parser_accepts_and_retains_executable_regular_blob_mode() {
        let output = b"100755 blob 0123456789012345678901234567890123456789\t.gwt/work/events/aaaaaaaa.jsonl\0";

        let entries = parse_tree_blob_entries(output).expect("executable regular blob");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mode, "100755");
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
