//! SPEC-2359 W-16 (FR-387): checkout-free blob access for the cross-machine
//! work events intake.
//!
//! The legacy batch API remains intact. Canonical event discovery uses one
//! persistent `cat-file --batch` child for every commit, recursively walks
//! shard trees without checkout, and reads each shared tree/selected blob oid
//! once.

use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::Stdio,
    sync::Arc,
};

use gwt_core::{GwtError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeBlobEntry {
    pub path: String,
    pub oid: String,
    pub mode: String,
}

/// One legacy or canonical Work-event blob resolved from a commit tree.
///
/// `content` is shared by oid across every returned commit, so many refs that
/// contain the same immutable event do not multiply payload allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkEventBlob {
    pub path: String,
    pub oid: String,
    /// Present when the caller selected this oid for payload reading. If any
    /// descriptor selects a shared oid, every descriptor for that oid receives
    /// the same allocation.
    pub content: Option<Arc<[u8]>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawTreeEntryKind {
    Tree,
    Blob,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawTreeEntry {
    name: Vec<u8>,
    oid: String,
    mode: String,
    kind: RawTreeEntryKind,
}

#[derive(Debug)]
struct BatchObject {
    oid: String,
    kind: String,
    content: Vec<u8>,
}

/// Checkout-free Work-event identity discovered for one input commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkEventBlobDescriptor {
    pub path: String,
    pub oid: String,
}

/// Discover the legacy Work-event log plus every recursively nested canonical
/// event blob for each commit, then selectively read payloads, in one
/// persistent `git cat-file --batch` process.
///
/// Results preserve `commits` input order. A valid commit without either path
/// produces an empty entry. Tree and selected blob oids are read once and
/// fanned out to every commit that shares them. Symlinks, gitlinks, unsupported modes,
/// malformed tree objects, missing referenced objects, and malformed batch
/// responses fail closed. `select_oids` runs after the complete descriptor
/// matrix is known and returns the payload oids needed by the caller. This lets
/// intake select all payloads for a rebuild, only additions for an incremental
/// pass, or none for an unchanged pass without another process.
pub fn work_event_blobs_batch<F>(
    repo_path: &Path,
    commits: &[String],
    legacy_path: &str,
    event_store_path: &str,
    select_oids: F,
) -> Result<Vec<Vec<WorkEventBlob>>>
where
    F: FnOnce(&[Vec<WorkEventBlobDescriptor>]) -> HashSet<String>,
{
    work_event_blobs_batch_with_hooks(
        repo_path,
        commits,
        legacy_path,
        event_store_path,
        select_oids,
        || {},
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn work_event_blobs_batch_with_hooks<F, S, Q>(
    repo_path: &Path,
    commits: &[String],
    legacy_path: &str,
    event_store_path: &str,
    select_oids: F,
    before_spawn: S,
    mut before_query: Q,
) -> Result<Vec<Vec<WorkEventBlob>>>
where
    F: FnOnce(&[Vec<WorkEventBlobDescriptor>]) -> HashSet<String>,
    S: FnOnce(),
    Q: FnMut(&str),
{
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let (parent_path, legacy_name, event_store_name) =
        split_work_event_tree_paths(legacy_path, event_store_path)?;
    for commit in commits {
        validate_batch_atom(commit, "commit")?;
    }

    let mut command = gwt_core::process::hidden_command("git");
    command
        .args(["cat-file", "--batch"])
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    before_spawn();
    let mut child = command
        .spawn()
        .map_err(|error| GwtError::Git(format!("cat-file --batch: {error}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| GwtError::Git("cat-file --batch: stdin unavailable".to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| GwtError::Git("cat-file --batch: stdout unavailable".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| GwtError::Git("cat-file --batch: stderr unavailable".to_string()))?;
    let stderr_reader = std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let mut stdout = BufReader::new(stdout);

    let result = work_event_blobs_from_batch(
        &mut stdin,
        &mut stdout,
        commits,
        &parent_path,
        &legacy_name,
        &event_store_name,
        legacy_path,
        event_store_path,
        select_oids,
        &mut before_query,
    );
    drop(stdin);
    let status = child
        .wait()
        .map_err(|error| GwtError::Git(format!("cat-file --batch wait: {error}")))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| GwtError::Git("cat-file --batch: stderr reader panicked".to_string()))?;
    if let Err(error) = result {
        return Err(error);
    }
    if !status.success() {
        return Err(GwtError::Git(format!(
            "cat-file --batch exited {status}: {}",
            String::from_utf8_lossy(&stderr).trim()
        )));
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn work_event_blobs_from_batch<W: Write, R: BufRead, F, Q>(
    stdin: &mut W,
    stdout: &mut R,
    commits: &[String],
    parent_path: &str,
    legacy_name: &str,
    event_store_name: &str,
    legacy_path: &str,
    event_store_path: &str,
    select_oids: F,
    before_query: &mut Q,
) -> Result<Vec<Vec<WorkEventBlob>>>
where
    F: FnOnce(&[Vec<WorkEventBlobDescriptor>]) -> HashSet<String>,
    Q: FnMut(&str),
{
    let mut unique_commits = Vec::new();
    let mut seen_commits = HashSet::new();
    for commit in commits {
        if seen_commits.insert(commit.clone()) {
            unique_commits.push(commit.clone());
        }
    }

    // Distinguish a missing commit (fatal) from a missing event path (empty),
    // and retain each commit's root tree oid so shared trees can be queried by
    // oid exactly once instead of once per `<commit>:<path>` expression.
    let commit_responses = query_batch_objects(stdin, stdout, &unique_commits, before_query)?;
    let mut root_tree_by_commit = HashMap::<String, Option<String>>::new();
    for (commit, response) in unique_commits.iter().zip(commit_responses) {
        let object = response
            .ok_or_else(|| GwtError::Git(format!("cat-file --batch: missing commit {commit}")))?;
        if object.kind != "commit" {
            return Err(GwtError::Git(format!(
                "cat-file --batch: expected commit {commit}, got {}",
                object.kind
            )));
        }
        let hash_bytes = object_hash_bytes(&object.oid)?;
        let root_tree = parse_commit_tree_oid(&object.content, hash_bytes)?;
        root_tree_by_commit.insert(commit.clone(), Some(root_tree));
    }

    let mut tree_entries = HashMap::<String, Vec<RawTreeEntry>>::new();
    let parent_tree_by_commit = resolve_tree_path_by_commit(
        stdin,
        stdout,
        &root_tree_by_commit,
        parent_path,
        &mut tree_entries,
        before_query,
    )?;
    load_tree_objects(
        stdin,
        stdout,
        parent_tree_by_commit.values().flatten(),
        &mut tree_entries,
        before_query,
    )?;

    let mut descriptors_by_commit = HashMap::<String, Vec<WorkEventBlobDescriptor>>::new();
    let mut event_roots_by_commit = HashMap::<String, String>::new();

    for commit in &unique_commits {
        let Some(parent_oid) = parent_tree_by_commit.get(commit).and_then(Option::as_ref) else {
            descriptors_by_commit.insert(commit.clone(), Vec::new());
            continue;
        };
        let entries = tree_entries.get(parent_oid).ok_or_else(|| {
            GwtError::Git(format!("cat-file --batch: unread tree object {parent_oid}"))
        })?;
        let mut descriptors = Vec::new();
        for entry in entries {
            if entry.name == legacy_name.as_bytes() {
                if entry.kind != RawTreeEntryKind::Blob {
                    return Err(GwtError::Git(format!(
                        "work event legacy path is not a regular blob: {legacy_path}"
                    )));
                }
                descriptors.push(WorkEventBlobDescriptor {
                    path: legacy_path.to_string(),
                    oid: entry.oid.clone(),
                });
            } else if entry.name == event_store_name.as_bytes() {
                if entry.kind != RawTreeEntryKind::Tree {
                    return Err(GwtError::Git(format!(
                        "work event store path is not a tree: {event_store_path}"
                    )));
                }
                event_roots_by_commit.insert(commit.clone(), entry.oid.clone());
            }
        }
        descriptors_by_commit.insert(commit.clone(), descriptors);
    }

    let mut discovered_tree_oids = tree_entries.keys().cloned().collect::<HashSet<_>>();
    let mut pending_tree_oids = HashSet::new();
    for oid in event_roots_by_commit.values() {
        if discovered_tree_oids.insert(oid.clone()) {
            pending_tree_oids.insert(oid.clone());
        }
    }
    while !pending_tree_oids.is_empty() {
        let mut oids = pending_tree_oids.drain().collect::<Vec<_>>();
        oids.sort();
        let responses = query_batch_objects(stdin, stdout, &oids, before_query)?;
        for (oid, response) in oids.into_iter().zip(responses) {
            let object = response.ok_or_else(|| {
                GwtError::Git(format!("cat-file --batch: missing tree object {oid}"))
            })?;
            if object.oid != oid || object.kind != "tree" {
                return Err(GwtError::Git(format!(
                    "cat-file --batch: expected tree {oid}, got {} {}",
                    object.oid, object.kind
                )));
            }
            let entries =
                parse_raw_tree_entries_allow_unsafe(&object.content, object_hash_bytes(&oid)?)?;
            reject_unsafe_tree_entries(&entries, event_store_path)?;
            for entry in &entries {
                if entry.kind == RawTreeEntryKind::Tree
                    && discovered_tree_oids.insert(entry.oid.clone())
                {
                    pending_tree_oids.insert(entry.oid.clone());
                }
            }
            tree_entries.insert(oid, entries);
        }
    }

    for commit in &unique_commits {
        let Some(root_oid) = event_roots_by_commit.get(commit) else {
            continue;
        };
        let descriptors = descriptors_by_commit.entry(commit.clone()).or_default();
        let mut stack = vec![(root_oid.clone(), event_store_path.to_string(), 0usize)];
        while let Some((tree_oid, prefix, depth)) = stack.pop() {
            if depth > 64 {
                return Err(GwtError::Git(format!(
                    "work event tree exceeds maximum depth below {event_store_path}"
                )));
            }
            let entries = tree_entries.get(&tree_oid).ok_or_else(|| {
                GwtError::Git(format!("cat-file --batch: unread tree object {tree_oid}"))
            })?;
            reject_unsafe_tree_entries(entries, &prefix)?;
            for entry in entries.iter().rev() {
                let name = std::str::from_utf8(&entry.name).map_err(|error| {
                    GwtError::Git(format!(
                        "work event tree contains an invalid UTF-8 name below {prefix}: {error}"
                    ))
                })?;
                let path = format!("{prefix}/{name}");
                match entry.kind {
                    RawTreeEntryKind::Tree => {
                        stack.push((entry.oid.clone(), path, depth + 1));
                    }
                    RawTreeEntryKind::Blob => descriptors.push(WorkEventBlobDescriptor {
                        path,
                        oid: entry.oid.clone(),
                    }),
                    RawTreeEntryKind::Unsupported => {
                        return Err(GwtError::Git(format!(
                            "work event tree contains unsupported mode {} at {path}",
                            entry.mode
                        )))
                    }
                }
            }
        }
        descriptors.sort_by(|left, right| left.path.cmp(&right.path));
    }

    let descriptor_matrix = commits
        .iter()
        .map(|commit| {
            descriptors_by_commit
                .get(commit)
                .cloned()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let available_blob_oids = descriptor_matrix
        .iter()
        .flatten()
        .map(|descriptor| descriptor.oid.clone())
        .collect::<HashSet<_>>();
    let selected_blob_oids = select_oids(&descriptor_matrix);
    if let Some(unknown) = selected_blob_oids.difference(&available_blob_oids).next() {
        return Err(GwtError::Git(format!(
            "work event payload selector returned unknown oid {unknown}"
        )));
    }
    let mut unique_blob_oids = selected_blob_oids.into_iter().collect::<Vec<_>>();
    unique_blob_oids.sort();
    let blob_responses = query_batch_objects(stdin, stdout, &unique_blob_oids, before_query)?;
    let mut blob_contents = HashMap::<String, Arc<[u8]>>::new();
    for (oid, response) in unique_blob_oids.into_iter().zip(blob_responses) {
        let object = response
            .ok_or_else(|| GwtError::Git(format!("cat-file --batch: missing blob object {oid}")))?;
        if object.oid != oid || object.kind != "blob" {
            return Err(GwtError::Git(format!(
                "cat-file --batch: expected blob {oid}, got {} {}",
                object.oid, object.kind
            )));
        }
        blob_contents.insert(oid, Arc::from(object.content.into_boxed_slice()));
    }

    descriptor_matrix
        .iter()
        .map(|descriptors| {
            descriptors
                .iter()
                .map(|descriptor| {
                    Ok(WorkEventBlob {
                        path: descriptor.path.clone(),
                        oid: descriptor.oid.clone(),
                        content: blob_contents.get(&descriptor.oid).cloned(),
                    })
                })
                .collect()
        })
        .collect()
}

fn parse_commit_tree_oid(content: &[u8], expected_oid_bytes: usize) -> Result<String> {
    let header_end = content
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| GwtError::Git("commit object has no tree header terminator".to_string()))?;
    let header = std::str::from_utf8(&content[..header_end]).map_err(|error| {
        GwtError::Git(format!("commit object has invalid tree header: {error}"))
    })?;
    let oid = header.strip_prefix("tree ").ok_or_else(|| {
        GwtError::Git("commit object does not begin with a tree header".to_string())
    })?;
    if object_hash_bytes(oid)? != expected_oid_bytes {
        return Err(GwtError::Git(format!(
            "commit tree oid width does not match commit oid: {oid}"
        )));
    }
    Ok(oid.to_ascii_lowercase())
}

fn resolve_tree_path_by_commit<W: Write, R: BufRead, Q: FnMut(&str)>(
    stdin: &mut W,
    stdout: &mut R,
    root_tree_by_commit: &HashMap<String, Option<String>>,
    path: &str,
    tree_entries: &mut HashMap<String, Vec<RawTreeEntry>>,
    before_query: &mut Q,
) -> Result<HashMap<String, Option<String>>> {
    let mut current = root_tree_by_commit.clone();
    let mut resolved_prefix = String::new();
    for component in path.split('/') {
        load_tree_objects(
            stdin,
            stdout,
            current.values().flatten(),
            tree_entries,
            before_query,
        )?;
        if !resolved_prefix.is_empty() {
            resolved_prefix.push('/');
        }
        resolved_prefix.push_str(component);
        let mut next = HashMap::with_capacity(current.len());
        for (commit, current_oid) in current {
            let Some(current_oid) = current_oid else {
                next.insert(commit, None);
                continue;
            };
            let entries = tree_entries.get(&current_oid).ok_or_else(|| {
                GwtError::Git(format!(
                    "cat-file --batch: unread tree object {current_oid}"
                ))
            })?;
            let Some(entry) = entries
                .iter()
                .find(|entry| entry.name == component.as_bytes())
            else {
                next.insert(commit, None);
                continue;
            };
            if entry.kind != RawTreeEntryKind::Tree {
                return Err(GwtError::Git(format!(
                    "work event managed path is not a tree: {resolved_prefix}"
                )));
            }
            next.insert(commit, Some(entry.oid.clone()));
        }
        current = next;
    }
    Ok(current)
}

fn load_tree_objects<'a, W, R, Q, I>(
    stdin: &mut W,
    stdout: &mut R,
    oids: I,
    tree_entries: &mut HashMap<String, Vec<RawTreeEntry>>,
    before_query: &mut Q,
) -> Result<()>
where
    W: Write,
    R: BufRead,
    Q: FnMut(&str),
    I: IntoIterator<Item = &'a String>,
{
    let mut pending = oids
        .into_iter()
        .filter(|oid| !tree_entries.contains_key(*oid))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    pending.sort();
    let responses = query_batch_objects(stdin, stdout, &pending, before_query)?;
    for (oid, response) in pending.into_iter().zip(responses) {
        let object = response
            .ok_or_else(|| GwtError::Git(format!("cat-file --batch: missing tree object {oid}")))?;
        if object.oid != oid || object.kind != "tree" {
            return Err(GwtError::Git(format!(
                "cat-file --batch: expected tree {oid}, got {} {}",
                object.oid, object.kind
            )));
        }
        let entries =
            parse_raw_tree_entries_allow_unsafe(&object.content, object_hash_bytes(&oid)?)?;
        tree_entries.insert(oid, entries);
    }
    Ok(())
}

fn reject_unsafe_tree_entries(entries: &[RawTreeEntry], path: &str) -> Result<()> {
    for entry in entries {
        if entry.kind == RawTreeEntryKind::Unsupported {
            return Err(GwtError::Git(format!(
                "work event tree contains unsupported mode {} below {path}",
                entry.mode
            )));
        }
        let name = std::str::from_utf8(&entry.name).map_err(|error| {
            GwtError::Git(format!(
                "work event tree contains an invalid UTF-8 name below {path}: {error}"
            ))
        })?;
        if name.is_empty()
            || matches!(name, "." | "..")
            || name.bytes().any(|byte| byte == b'/' || byte == b'\\')
        {
            return Err(GwtError::Git(format!(
                "work event tree contains an invalid entry name {name:?} below {path}"
            )));
        }
    }
    Ok(())
}

fn split_work_event_tree_paths(
    legacy_path: &str,
    event_store_path: &str,
) -> Result<(String, String, String)> {
    validate_git_tree_path(legacy_path, "legacy path")?;
    validate_git_tree_path(event_store_path, "event store path")?;
    let (legacy_parent, legacy_name) = legacy_path.rsplit_once('/').ok_or_else(|| {
        GwtError::Git(format!(
            "work event legacy path has no parent: {legacy_path}"
        ))
    })?;
    let (store_parent, store_name) = event_store_path.rsplit_once('/').ok_or_else(|| {
        GwtError::Git(format!(
            "work event store path has no parent: {event_store_path}"
        ))
    })?;
    if legacy_parent != store_parent || legacy_name == store_name {
        return Err(GwtError::Git(format!(
            "work event paths must be distinct siblings: {legacy_path}, {event_store_path}"
        )));
    }
    Ok((
        legacy_parent.to_string(),
        legacy_name.to_string(),
        store_name.to_string(),
    ))
}

fn validate_git_tree_path(path: &str, label: &str) -> Result<()> {
    validate_batch_atom(path, label)?;
    if path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(GwtError::Git(format!(
            "cat-file --batch: invalid {label}: {path:?}"
        )));
    }
    Ok(())
}

fn validate_batch_atom(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| byte == 0 || byte == b'\n' || byte == b'\r')
    {
        return Err(GwtError::Git(format!(
            "cat-file --batch: invalid {label}: {value:?}"
        )));
    }
    Ok(())
}

fn query_batch_objects<W: Write, R: BufRead, Q: FnMut(&str)>(
    stdin: &mut W,
    stdout: &mut R,
    specs: &[String],
    before_query: &mut Q,
) -> Result<Vec<Option<BatchObject>>> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let mut responses = Vec::with_capacity(specs.len());
    for spec in specs {
        validate_batch_atom(spec, "object spec")?;
        before_query(spec);
        stdin
            .write_all(spec.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|error| GwtError::Git(format!("cat-file --batch stdin: {error}")))?;
        stdin
            .flush()
            .map_err(|error| GwtError::Git(format!("cat-file --batch stdin flush: {error}")))?;
        responses.push(read_batch_object(stdout, spec)?);
    }
    Ok(responses)
}

fn read_batch_object<R: BufRead>(reader: &mut R, spec: &str) -> Result<Option<BatchObject>> {
    let mut header = Vec::new();
    let read = reader
        .read_until(b'\n', &mut header)
        .map_err(|error| GwtError::Git(format!("cat-file --batch header: {error}")))?;
    if read == 0 || header.last() != Some(&b'\n') {
        return Err(GwtError::Git(format!(
            "cat-file --batch: missing header terminator for {spec}"
        )));
    }
    header.pop();
    if header.last() == Some(&b'\r') {
        header.pop();
    }
    let header = std::str::from_utf8(&header)
        .map_err(|error| GwtError::Git(format!("cat-file --batch invalid header: {error}")))?;
    if header == format!("{spec} missing") {
        return Ok(None);
    }
    let mut fields = header.split_whitespace();
    let oid = fields.next().unwrap_or_default();
    let kind = fields.next().unwrap_or_default();
    let size = fields.next().and_then(|value| value.parse::<usize>().ok());
    if fields.next().is_some() || !matches!(kind, "blob" | "tree" | "commit") || size.is_none() {
        return Err(GwtError::Git(format!(
            "cat-file --batch: unexpected header for {spec}: {header}"
        )));
    }
    object_hash_bytes(oid)?;
    let size = size.unwrap();
    let mut content = vec![0; size];
    reader
        .read_exact(&mut content)
        .map_err(|error| GwtError::Git(format!("cat-file --batch content for {spec}: {error}")))?;
    let mut terminator = [0u8; 1];
    reader.read_exact(&mut terminator).map_err(|error| {
        GwtError::Git(format!(
            "cat-file --batch content terminator for {spec}: {error}"
        ))
    })?;
    if terminator[0] != b'\n' {
        return Err(GwtError::Git(format!(
            "cat-file --batch: invalid content terminator for {spec}"
        )));
    }
    Ok(Some(BatchObject {
        oid: oid.to_ascii_lowercase(),
        kind: kind.to_string(),
        content,
    }))
}

fn object_hash_bytes(oid: &str) -> Result<usize> {
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GwtError::Git(format!(
            "cat-file --batch: invalid object oid {oid:?}"
        )));
    }
    Ok(oid.len() / 2)
}

#[cfg(test)]
fn parse_raw_tree_entries(output: &[u8], oid_bytes: usize) -> Result<Vec<RawTreeEntry>> {
    let entries = parse_raw_tree_entries_allow_unsafe(output, oid_bytes)?;
    reject_unsafe_tree_entries(&entries, "raw tree")?;
    Ok(entries)
}

fn parse_raw_tree_entries_allow_unsafe(
    output: &[u8],
    oid_bytes: usize,
) -> Result<Vec<RawTreeEntry>> {
    if !matches!(oid_bytes, 20 | 32) {
        return Err(GwtError::Git(format!(
            "cat-file --batch: unsupported raw tree oid width {oid_bytes}"
        )));
    }
    let mut cursor = 0usize;
    let mut entries = Vec::new();
    let mut names = HashSet::new();
    while cursor < output.len() {
        let mode_end = output[cursor..]
            .iter()
            .position(|byte| *byte == b' ')
            .map(|offset| cursor + offset)
            .ok_or_else(|| GwtError::Git("raw tree: missing mode terminator".to_string()))?;
        let mode = std::str::from_utf8(&output[cursor..mode_end])
            .map_err(|error| GwtError::Git(format!("raw tree: invalid mode: {error}")))?;
        let kind = match mode {
            "40000" => RawTreeEntryKind::Tree,
            "100644" | "100755" => RawTreeEntryKind::Blob,
            "120000" | "160000" => RawTreeEntryKind::Unsupported,
            _ => return Err(GwtError::Git(format!("raw tree: unsupported mode {mode}"))),
        };
        let name_start = mode_end + 1;
        let name_end = output[name_start..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| name_start + offset)
            .ok_or_else(|| GwtError::Git("raw tree: missing name terminator".to_string()))?;
        let name = &output[name_start..name_end];
        if name.is_empty() || matches!(name, b"." | b"..") || name.contains(&b'/') {
            return Err(GwtError::Git(format!(
                "raw tree: invalid entry name {:?}",
                String::from_utf8_lossy(name)
            )));
        }
        if !names.insert(name.to_vec()) {
            return Err(GwtError::Git(format!(
                "raw tree: duplicate entry name {:?}",
                String::from_utf8_lossy(name)
            )));
        }
        let oid_start = name_end + 1;
        let oid_end = oid_start
            .checked_add(oid_bytes)
            .ok_or_else(|| GwtError::Git("raw tree: object oid boundary overflow".to_string()))?;
        if oid_end > output.len() {
            return Err(GwtError::Git(format!(
                "raw tree: truncated object oid for {:?}",
                String::from_utf8_lossy(name)
            )));
        }
        let oid = output[oid_start..oid_end]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        entries.push(RawTreeEntry {
            name: name.to_vec(),
            oid,
            mode: mode.to_string(),
            kind,
        });
        cursor = oid_end;
    }
    Ok(entries)
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

    fn distinct_commit_oids_with_head_tree(repo: &Path, count: usize) -> Vec<String> {
        let tree = gwt_core::process::hidden_command("git")
            .args(["rev-parse", "HEAD^{tree}"])
            .current_dir(repo)
            .output()
            .expect("head tree");
        assert!(tree.status.success());
        let tree = String::from_utf8_lossy(&tree.stdout).trim().to_string();
        let mut paths = String::new();
        for index in 0..count {
            let name = format!("commit-object-{index}.txt");
            std::fs::write(
                repo.join(&name),
                format!(
                    "tree {tree}\nauthor Test User <test@example.com> {} +0000\ncommitter Test User <test@example.com> {} +0000\n\nempty {index}\n",
                    1_700_000_000 + index,
                    1_700_000_000 + index,
                ),
            )
            .expect("commit object source");
            paths.push_str(&name);
            paths.push('\n');
        }
        let output = gwt_core::process::run_git_logged_with_stdin(
            &["hash-object", "-t", "commit", "-w", "--stdin-paths"],
            Some(repo),
            paths.as_bytes(),
        )
        .expect("batch commit object creation");
        assert!(
            output.status.success(),
            "batch commit creation: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let commits = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(commits.len(), count);
        commits
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
    fn persistent_batch_reads_legacy_flat_and_nested_event_blobs_without_checkout() {
        let dir = init_repo();
        let repo = dir.path();
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "-b", "work/persistent-batch"])
            .current_dir(repo));
        std::fs::create_dir_all(repo.join(".gwt/work/events/aa/deep")).expect("nested event store");
        std::fs::write(repo.join(".gwt/work/events.jsonl"), b"legacy\n").expect("legacy");
        std::fs::write(repo.join(".gwt/work/events/flat.jsonl"), b"flat\n").expect("flat shard");
        std::fs::write(
            repo.join(".gwt/work/events/aa/deep/nested.jsonl"),
            b"nested\n",
        )
        .expect("nested shard");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work"])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "persistent batch events"])
            .current_dir(repo));
        let with_events = head_sha(repo);
        run(gwt_core::process::hidden_command("git")
            .args(["checkout", "main"])
            .current_dir(repo));
        let without_events = head_sha(repo);

        let commits = vec![with_events, without_events];
        let blobs = work_event_blobs_batch(
            repo,
            &commits,
            ".gwt/work/events.jsonl",
            ".gwt/work/events",
            |descriptors| {
                descriptors
                    .iter()
                    .flatten()
                    .map(|descriptor| descriptor.oid.clone())
                    .collect()
            },
        )
        .expect("persistent batch discovery/read");

        assert_eq!(blobs.len(), 2);
        assert_eq!(
            blobs[0]
                .iter()
                .map(|blob| (blob.path.as_str(), blob.content.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (".gwt/work/events.jsonl", Some(b"legacy\n".as_slice()),),
                (
                    ".gwt/work/events/aa/deep/nested.jsonl",
                    Some(b"nested\n".as_slice()),
                ),
                (".gwt/work/events/flat.jsonl", Some(b"flat\n".as_slice()),),
            ]
        );
        assert!(blobs[1].is_empty(), "missing paths are an empty source");

        let descriptors_only = work_event_blobs_batch(
            repo,
            &commits,
            ".gwt/work/events.jsonl",
            ".gwt/work/events",
            |_| HashSet::new(),
        )
        .expect("unchanged intake can skip every payload");
        assert!(descriptors_only
            .iter()
            .flatten()
            .all(|blob| blob.content.is_none()));
    }

    #[test]
    fn persistent_batch_fans_shared_tree_and_blob_oids_across_many_distinct_commits() {
        let dir = init_repo();
        let repo = dir.path();
        std::fs::create_dir_all(repo.join(".gwt/work/events/aa")).expect("event store");
        std::fs::write(repo.join(".gwt/work/events/aa/shared.jsonl"), b"shared\n")
            .expect("shared shard");
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events"])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "shared event tree"])
            .current_dir(repo));

        let commits = distinct_commit_oids_with_head_tree(repo, 25);

        let shared_tree_oids = [
            "HEAD^{tree}",
            "HEAD:.gwt",
            "HEAD:.gwt/work",
            "HEAD:.gwt/work/events",
        ]
        .map(|spec| {
            let output = gwt_core::process::hidden_command("git")
                .args(["rev-parse", spec])
                .current_dir(repo)
                .output()
                .expect("resolve shared tree oid");
            assert!(output.status.success(), "{spec}");
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        });
        let spawn_count = std::sync::atomic::AtomicUsize::new(0);
        let query_counts = std::sync::Mutex::new(HashMap::<String, usize>::new());
        let blobs = work_event_blobs_batch_with_hooks(
            repo,
            &commits,
            ".gwt/work/events.jsonl",
            ".gwt/work/events",
            |descriptors| HashSet::from([descriptors[0][0].oid.clone()]),
            || {
                spawn_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
            |spec| {
                *query_counts
                    .lock()
                    .expect("query count lock")
                    .entry(spec.to_string())
                    .or_default() += 1;
            },
        )
        .expect("one persistent batch handles every distinct commit");

        assert_eq!(spawn_count.load(std::sync::atomic::Ordering::Relaxed), 1);
        let query_counts = query_counts.into_inner().expect("query count lock");
        for oid in shared_tree_oids {
            assert_eq!(
                query_counts.get(&oid),
                Some(&1),
                "shared tree {oid} must be requested exactly once: {query_counts:?}"
            );
        }
        assert_eq!(blobs.len(), commits.len());
        assert!(blobs.iter().all(|entries| entries.len() == 1));
        for entries in blobs.iter().skip(1) {
            assert_eq!(entries[0].oid, blobs[0][0].oid);
            assert!(
                std::sync::Arc::ptr_eq(
                    entries[0]
                        .content
                        .as_ref()
                        .expect("selected shared content"),
                    blobs[0][0]
                        .content
                        .as_ref()
                        .expect("selected first content"),
                ),
                "a shared blob oid must be read and allocated once"
            );
        }
    }

    #[test]
    fn persistent_batch_ignores_unrelated_symlink_entries_above_the_managed_tree() {
        let dir = init_repo();
        let repo = dir.path();
        std::fs::create_dir_all(repo.join(".gwt/work/events/aa")).expect("event store");
        std::fs::write(repo.join(".gwt/work/events/aa/event.jsonl"), b"event\n")
            .expect("event shard");
        std::fs::write(repo.join("link-target.txt"), b"outside-target\n").expect("link payload");
        let blob = gwt_core::process::hidden_command("git")
            .args(["hash-object", "-w", "link-target.txt"])
            .current_dir(repo)
            .output()
            .expect("hash symlink payload");
        assert!(blob.status.success());
        let blob = String::from_utf8_lossy(&blob.stdout).trim().to_string();
        run(gwt_core::process::hidden_command("git")
            .args(["add", ".gwt/work/events"])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-index",
                "--add",
                "--cacheinfo",
                "120000",
                &blob,
                "external-link",
            ])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args([
                "update-index",
                "--add",
                "--cacheinfo",
                "120000",
                &blob,
                ".gwt/work/unrelated-link",
            ])
            .current_dir(repo));
        run(gwt_core::process::hidden_command("git")
            .args(["commit", "-m", "managed tree with unrelated root symlink"])
            .current_dir(repo));
        let commit = head_sha(repo);

        let blobs = work_event_blobs_batch(
            repo,
            &[commit],
            ".gwt/work/events.jsonl",
            ".gwt/work/events",
            |descriptors| {
                descriptors
                    .iter()
                    .flatten()
                    .map(|item| item.oid.clone())
                    .collect()
            },
        )
        .expect("unrelated root modes must not invalidate managed traversal");

        assert_eq!(blobs[0].len(), 1);
        assert_eq!(blobs[0][0].path, ".gwt/work/events/aa/event.jsonl");
    }

    #[test]
    fn raw_tree_parser_supports_sha1_and_sha256_and_rejects_unsafe_modes() {
        fn record(mode: &str, name: &str, oid: &[u8]) -> Vec<u8> {
            let mut bytes = format!("{mode} {name}").into_bytes();
            bytes.push(0);
            bytes.extend_from_slice(oid);
            bytes
        }

        let sha1 = parse_raw_tree_entries(&record("100644", "event.jsonl", &[0x11; 20]), 20)
            .expect("sha1 tree entry");
        assert_eq!(sha1[0].oid, "11".repeat(20));
        let sha256 = parse_raw_tree_entries(&record("40000", "aa", &[0x22; 32]), 32)
            .expect("sha256 tree entry");
        assert_eq!(sha256[0].oid, "22".repeat(32));

        for mode in ["120000", "160000", "100600"] {
            let error = parse_raw_tree_entries(&record(mode, "unsafe", &[0x33; 20]), 20)
                .expect_err("unsafe tree modes fail closed");
            assert!(error.to_string().contains("unsupported mode"), "{error}");
        }
        let mut duplicate = record("40000", ".gwt", &[0x44; 20]);
        duplicate.extend(record("40000", ".gwt", &[0x55; 20]));
        let error = parse_raw_tree_entries_allow_unsafe(&duplicate, 20)
            .expect_err("duplicate tree entry names are malformed");
        assert!(
            error.to_string().contains("duplicate entry name"),
            "{error}"
        );
        let unrelated_backslash = record("100644", r"unrelated\name", &[0x66; 20]);
        let entries = parse_raw_tree_entries_allow_unsafe(&unrelated_backslash, 20)
            .expect("root traversal preserves legal unrelated backslashes");
        assert_eq!(entries[0].name, br"unrelated\name");
        assert!(parse_raw_tree_entries(&unrelated_backslash, 20).is_err());
        let mut unrelated_non_utf8 = b"100644 unrelated-".to_vec();
        unrelated_non_utf8.push(0xff);
        unrelated_non_utf8.push(0);
        unrelated_non_utf8.extend_from_slice(&[0x77; 20]);
        let entries = parse_raw_tree_entries_allow_unsafe(&unrelated_non_utf8, 20)
            .expect("root traversal preserves legal unrelated non-UTF-8 names");
        assert_eq!(entries[0].name.last(), Some(&0xff));
        assert!(parse_raw_tree_entries(&unrelated_non_utf8, 20).is_err());
        assert!(parse_raw_tree_entries(b"100644 missing-nul", 20).is_err());
        assert!(parse_raw_tree_entries(b"100644 truncated\0short", 20).is_err());
    }

    #[test]
    fn persistent_batch_parser_rejects_malformed_headers_and_payloads() {
        let oid = "1".repeat(40);
        for response in [
            format!("{oid} blob nope\n"),
            format!("{oid} unknown 0\n\n"),
            format!("{oid} blob 4\nabc"),
            format!("{oid} blob 3\nabc!"),
        ] {
            let error = read_batch_object(
                &mut std::io::Cursor::new(response.into_bytes()),
                "object-spec",
            )
            .expect_err("malformed batch response must fail closed");
            assert!(error.to_string().contains("cat-file --batch"), "{error}");
        }
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
