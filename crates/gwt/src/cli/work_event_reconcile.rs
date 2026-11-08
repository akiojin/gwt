//! Lossless recovery of the legacy cross-host successor tombstone collision.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use gwt_core::{
    paths::{
        gwt_home, gwt_project_dir_for_repo_path, gwt_repo_local_work_event_shard_path,
        gwt_repo_local_work_events_dir,
    },
    process::hidden_command,
    workspace_projection::{preserve_workspace_work_event_log_as_shards, WorkEvent},
};
use gwt_github::{ApiError, SpecOpsError};
use serde_json::json;
use sha2::{Digest, Sha256};

struct PreservedEvent {
    original: Vec<u8>,
    archive: PathBuf,
    replacement: Vec<u8>,
    replacement_archive: PathBuf,
    replacement_path: PathBuf,
}

struct Collision {
    relative_path: String,
    local: PreservedEvent,
    incoming: PreservedEvent,
}

pub(super) fn run(
    repo: &Path,
    reference: &str,
    dry_run: bool,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    reconcile(repo, reference, dry_run, out).map_err(|error| {
        SpecOpsError::from(ApiError::Network(format!(
            "workspace.events_reconcile: {error}"
        )))
    })
}

fn reconcile(repo: &Path, reference: &str, dry_run: bool, out: &mut String) -> io::Result<i32> {
    let root = String::from_utf8(git(repo, &["rev-parse", "--show-toplevel"])?).map_err(invalid)?;
    let root = PathBuf::from(root.trim_end_matches(['\r', '\n']));
    let repo = root.as_path();
    // Pin the already-fetched reference. No fetch, index edit, checkout or merge
    // belongs to this operation; in particular a dry run writes no plan file.
    let reference_sha = String::from_utf8(git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )?)
    .map_err(invalid)?
    .trim()
    .to_string();
    let index = git(
        repo,
        &["ls-files", "--stage", "-z", "--", ".gwt/work/events/"],
    )?;
    let mut tracked_blobs = BTreeMap::new();
    for entry in index
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let separator = entry
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| invalid("invalid Git index entry"))?;
        let blob = entry[..separator]
            .split(|byte| *byte == b' ')
            .nth(1)
            .ok_or_else(|| invalid("missing Git index blob"))?;
        tracked_blobs.insert(&entry[separator + 1..], blob);
    }
    let mut dirty = git(
        repo,
        &["diff", "--name-only", "-z", "--", ".gwt/work/events/"],
    )?;
    dirty.extend(git(
        repo,
        &[
            "diff",
            "--cached",
            "--name-only",
            "-z",
            "--",
            ".gwt/work/events/",
        ],
    )?);
    let dirty = dirty
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .collect::<BTreeSet<_>>();
    let tree = git(
        repo,
        &[
            "ls-tree",
            "-r",
            "-z",
            "--full-tree",
            &reference_sha,
            "--",
            ".gwt/work/events/",
        ],
    )?;
    let archive_dir = gwt_project_dir_for_repo_path(repo).join("work-event-reconcile");
    let archive_root = gwt_home();
    let source_root = repo.join(".gwt");
    let mut collisions = Vec::new();
    for entry in tree
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let separator = entry
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| invalid("invalid Git tree entry"))?;
        let (header, relative) = (&entry[..separator], &entry[separator + 1..]);
        if super::delivery_paths::classify_path(relative)
            != super::delivery_paths::DeliveryPath::WorkEventShard
        {
            continue;
        }
        let relative_path = std::str::from_utf8(relative).map_err(invalid)?.to_string();
        if dirty.contains(relative) {
            return Err(invalid(format!(
                "tracked shard cannot be reconciled: {relative_path}"
            )));
        }
        let fields = std::str::from_utf8(header)
            .map_err(invalid)?
            .split_whitespace()
            .collect::<Vec<_>>();
        if let Some(blob) = tracked_blobs.get(relative) {
            // Compare all historical tracked events in one index/tree scan,
            // without spawning cat-file once for each ordinary shard.
            if fields.get(2).map(|value| value.as_bytes()) == Some(*blob) {
                continue;
            }
            return Err(invalid(format!(
                "tracked shard cannot be reconciled: {relative_path}"
            )));
        }
        let source = repo.join(&relative_path);
        if !real_file(&source)? {
            continue;
        }
        validate_source(repo, &source)?;
        if fields.len() != 3 || fields[0] != "100644" || fields[1] != "blob" {
            return Err(invalid(format!(
                "incoming shard is not a regular non-executable file: {relative_path}"
            )));
        }
        let incoming = git(repo, &["cat-file", "blob", fields[2]])?;
        let local = fs::read(&source)?;
        if local == incoming {
            continue;
        }
        let local_event = legacy_tombstone(repo, &source, &local)?;
        let incoming_event = legacy_tombstone(repo, &source, &incoming)?;
        if local_event.work_item_id != incoming_event.work_item_id
            || local_event.related_work_item_id != incoming_event.related_work_item_id
            || local_event
                .execution_container
                .as_ref()
                .and_then(|value| value.branch.as_deref())
                != incoming_event
                    .execution_container
                    .as_ref()
                    .and_then(|value| value.branch.as_deref())
        {
            return Err(invalid(format!(
                "tombstones do not share the same Work, parent and branch: {relative_path}"
            )));
        }
        collisions.push(Collision {
            relative_path,
            local: prepare(repo, &archive_dir, local, local_event)?,
            incoming: prepare(repo, &archive_dir, incoming, incoming_event)?,
        });
    }

    // Refuse every known destination conflict before publishing or moving any
    // source. Retrying after an interrupted apply accepts only identical bytes.
    for collision in &collisions {
        for event in [&collision.local, &collision.incoming] {
            check_destination(&event.archive, &event.original, &archive_root)?;
            check_destination(
                &event.replacement_archive,
                &event.replacement,
                &archive_root,
            )?;
            check_destination(&event.replacement_path, &event.replacement, &source_root)?;
        }
    }
    if !dry_run {
        for collision in &collisions {
            for event in [&collision.local, &collision.incoming] {
                archive(&event.archive, &event.original, &archive_root)?;
                archive(
                    &event.replacement_archive,
                    &event.replacement,
                    &archive_root,
                )?;
                preserve_workspace_work_event_log_as_shards(
                    &event.replacement_archive,
                    &gwt_repo_local_work_events_dir(repo),
                )
                .map_err(invalid)?;
                if fs::read(&event.replacement_path)? != event.replacement {
                    return Err(invalid("replacement shard readback mismatch"));
                }
            }
            let source = repo.join(&collision.relative_path);
            validate_source(repo, &source)?;
            if fs::read(&source)? != collision.local.original {
                return Err(invalid("local source changed during reconciliation"));
            }
            if !git(repo, &["ls-files", "-z", "--", &collision.relative_path])?.is_empty() {
                return Err(invalid("source became tracked during reconciliation"));
            }
            // The exact source bytes have been synced and read back from the
            // content archive; only this untracked legacy path is removed.
            fs::remove_file(&source)?;
            sync_directory(source.parent().expect("canonical shard bucket"))?;
        }
    }
    let result = json!({
        "dry_run": dry_run,
        "reference": reference,
        "reference_sha": reference_sha,
        "reconciled": if dry_run { 0 } else { collisions.len() },
        "collisions": collisions.iter().map(|collision| json!({
            "path": collision.relative_path,
            "local_archive": collision.local.archive,
            "incoming_archive": collision.incoming.archive,
            "local_shard": collision.local.replacement_path,
            "incoming_shard": collision.incoming.replacement_path,
        })).collect::<Vec<_>>()
    });
    out.push_str(&serde_json::to_string_pretty(&result).map_err(invalid)?);
    out.push('\n');
    Ok(0)
}

fn legacy_tombstone(repo: &Path, source: &Path, bytes: &[u8]) -> io::Result<WorkEvent> {
    let event: WorkEvent = serde_json::from_slice(bytes).map_err(invalid)?;
    if !event.is_successor_supersession()
        || event.id != format!("superseded-{}", event.work_item_id)
        || event
            .execution_container
            .as_ref()
            .and_then(|value| value.branch.as_deref())
            .is_none_or(str::is_empty)
        || gwt_repo_local_work_event_shard_path(repo, &event.id) != source
    {
        return Err(invalid(format!(
            "source is not a canonical legacy successor tombstone: {}",
            source.display()
        )));
    }
    Ok(event)
}

fn prepare(
    repo: &Path,
    archive_dir: &Path,
    original: Vec<u8>,
    mut event: WorkEvent,
) -> io::Result<PreservedEvent> {
    event
        .bind_successor_supersession_identity()
        .map_err(invalid)?;
    let mut replacement = serde_json::to_vec(&event).map_err(invalid)?;
    replacement.push(b'\n');
    Ok(PreservedEvent {
        archive: archive_dir.join(format!("{:x}.jsonl", Sha256::digest(&original))),
        replacement_archive: archive_dir.join(format!("{:x}.jsonl", Sha256::digest(&replacement))),
        replacement_path: gwt_repo_local_work_event_shard_path(repo, &event.id),
        original,
        replacement,
    })
}

fn validate_source(repo: &Path, path: &Path) -> io::Result<()> {
    for ancestor in path
        .parent()
        .expect("canonical shard parent")
        .ancestors()
        .take_while(|path| *path != repo)
    {
        if !fs::symlink_metadata(ancestor)?.file_type().is_dir() {
            return Err(invalid("source directory is redirected"));
        }
    }
    if !real_file(path)? {
        return Err(invalid("source is missing"));
    }
    Ok(())
}

fn real_file(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(invalid(format!(
            "path is not a regular file: {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn check_destination(path: &Path, bytes: &[u8], managed_root: &Path) -> io::Result<()> {
    if !path.starts_with(managed_root) {
        return Err(invalid("destination is outside its managed root"));
    }
    // The configured HOME or repository root may legitimately be reached
    // through an OS alias (macOS /var is one). Inspect only the managed tail.
    for parent in path
        .parent()
        .expect("destination parent")
        .ancestors()
        .take_while(|parent| parent.starts_with(managed_root))
    {
        match fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(invalid(format!(
                    "destination directory is redirected: {}",
                    parent.display()
                )))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if real_file(path)? && fs::read(path)? != bytes {
        return Err(invalid(format!(
            "destination has different bytes: {}",
            path.display()
        )));
    }
    Ok(())
}

fn archive(path: &Path, bytes: &[u8], managed_root: &Path) -> io::Result<()> {
    check_destination(path, bytes, managed_root)?;
    let parent = path.parent().expect("archive parent");
    fs::create_dir_all(parent)?;
    check_destination(path, bytes, managed_root)?;
    match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            // A retry may encounter a complete file created before an earlier
            // sync failed. Flush it without changing or truncating its bytes.
            OpenOptions::new().write(true).open(path)?.sync_all()?;
        }
        Err(error) => return Err(error),
    }
    check_destination(path, bytes, managed_root)?;
    // The original is removed only after every newly created directory entry,
    // including .gwt itself, is durable as well as the file contents.
    for directory in parent
        .ancestors()
        .take_while(|directory| directory.starts_with(managed_root))
    {
        sync_directory(directory)?;
    }
    sync_directory(managed_root.parent().expect("managed root parent"))
}

fn git(repo: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let output = hidden_command("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}
