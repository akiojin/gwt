//! Model-free project index status projection (SPEC #1939 Phase 71 AS-32 /
//! FR-416, Issue #3772 T-IDX-435).
//!
//! Every status / probe entrypoint projects index health from on-disk
//! metadata only. It never ensures the Python runtime, never spawns a runner
//! process, and never creates or rewrites a byte below `~/.gwt/runtime`,
//! `~/.gwt/index`, or a legacy `$WORKTREE/.gwt/index`. Chroma collections are
//! therefore not opened; the projection trusts the verified generation
//! descriptors, manifests, and digests the runner published, and the runner
//! keeps deep verification for search and repair.
//!
//! The payload shape mirrors the runner's `status` action so
//! [`crate::index_worker::parse_scope_health`] and the aggregated view stay
//! unchanged for the GUI.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use gwt_core::index::view::{
    canonical_json_sha256, BaseGenerationDescriptor, FileIndexCompatibilityDescriptor,
    OverlayGenerationDescriptor, WorktreeViewDescriptor, WorktreeViewHead,
};
use serde_json::{json, Map, Value};

/// Repo-shared scopes every status payload reports, in emission order.
pub(crate) const REPO_SHARED_SCOPES: [&str; 4] = ["specs", "memory", "discussions", "board"];
const FILE_SCOPES: [&str; 2] = ["files", "files-docs"];
const GENERATIONS_DIR_SUFFIX: &str = ".gen";
const ACTIVE_POINTER_FILENAME: &str = "active.json";
const CHROMA_STORE_FILENAME: &str = "chroma.sqlite3";

/// Content hash of the bundled runner without touching the shared runtime
/// directory. Mirrors the `asset_hash` the runner reports for diagnostics.
pub(crate) fn runner_asset_hash_read_only() -> String {
    gwt_core::runtime::bundled_project_index_runner_hash()
}

/// Repo-shared scope health (`issues`, `specs`, `memory`, `discussions`,
/// `board`) keyed by scope name.
pub(crate) fn repo_shared_status(
    index_root: &Path,
    repo_hash: &str,
    issue_cache_root: &Path,
) -> Map<String, Value> {
    let repo_dir = index_root.join(repo_hash);
    let mut status = Map::new();
    status.insert(
        "issues".to_string(),
        issues_status(&repo_dir, issue_cache_root),
    );
    for scope in REPO_SHARED_SCOPES {
        status.insert(scope.to_string(), repo_scope_status(&repo_dir, scope));
    }
    status
}

/// Per-worktree `files` / `files-docs` health from the additive v2 layout
/// with the legacy per-worktree store as the fallback reader.
pub(crate) fn worktree_file_status(
    index_root: &Path,
    repo_hash: &str,
    worktree_hash: &str,
) -> Map<String, Value> {
    let repo_dir = index_root.join(repo_hash);
    let selection = select_v2_view(&repo_dir.join("file-index-v2"), repo_hash, worktree_hash);
    let mut status = Map::new();
    for scope in FILE_SCOPES {
        let value = match &selection.view {
            Some(view) => {
                let mut value = json!({
                    "exists": true,
                    "healthy": true,
                    "repair_required": selection.repair_required,
                    "document_count": view.visible_counts.get(scope).copied().unwrap_or(0),
                    "reason": selection.reason,
                    "legacy_residue_detected": false,
                    "last_repair_at": Value::Null,
                    "view_id": view.view_id,
                });
                if let Some(fallback) = selection.fallback_source {
                    value["fallback_source"] = Value::String(fallback.to_string());
                }
                value
            }
            None => {
                let mut legacy = legacy_worktree_scope_status(&repo_dir, worktree_hash, scope);
                let legacy_healthy = legacy["healthy"].as_bool().unwrap_or(false)
                    && !legacy["repair_required"].as_bool().unwrap_or(true);
                legacy["repair_required"] = Value::Bool(true);
                legacy["reason"] = Value::String(selection.reason.clone());
                if legacy_healthy {
                    legacy["healthy"] = Value::Bool(true);
                    legacy["fallback_source"] = Value::String("legacy".to_string());
                } else {
                    legacy["healthy"] = Value::Bool(false);
                }
                legacy
            }
        };
        status.insert(scope.to_string(), value);
    }
    status
}

fn health(healthy: bool, document_count: u64, reason: &str) -> Value {
    json!({
        "exists": healthy,
        "healthy": healthy,
        "repair_required": !healthy,
        "document_count": document_count,
        "reason": reason,
        "legacy_residue_detected": false,
        "last_repair_at": Value::Null,
    })
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ---------------------------------------------------------------------------
// Legacy generation pointer layout (`<scope>.gen/active.json`)
// ---------------------------------------------------------------------------

fn generations_root(db_path: &Path) -> PathBuf {
    let name = db_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    db_path
        .parent()
        .unwrap_or(db_path)
        .join(format!("{name}{GENERATIONS_DIR_SUFFIX}"))
}

fn read_active_pointer(db_path: &Path) -> Option<String> {
    let pointer = read_json(&generations_root(db_path).join(ACTIVE_POINTER_FILENAME))?;
    pointer
        .get("generation")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn active_pointer_corrupt(db_path: &Path) -> bool {
    let pointer_path = generations_root(db_path).join(ACTIVE_POINTER_FILENAME);
    if !pointer_path.is_file() {
        return false;
    }
    match read_active_pointer(db_path) {
        Some(generation) => !generations_root(db_path).join(generation).is_dir(),
        None => true,
    }
}

fn active_store(db_path: &Path) -> PathBuf {
    if let Some(generation) = read_active_pointer(db_path) {
        let generation_dir = generations_root(db_path).join(generation);
        if generation_dir.is_dir() {
            return generation_dir;
        }
    }
    db_path.to_path_buf()
}

fn manifest_entry_count(manifest_path: &Path) -> Option<u64> {
    let payload = read_json(manifest_path)?;
    let entries = match &payload {
        Value::Array(entries) => entries,
        Value::Object(object) => object.get("entries")?.as_array()?,
        _ => return None,
    };
    Some(entries.len() as u64)
}

fn issues_status(repo_dir: &Path, issue_cache_root: &Path) -> Value {
    let db_path = repo_dir.join("issues");
    let meta_path = db_path.join("meta.json");
    let meta = read_json(&meta_path);
    let exists =
        active_store(&db_path).join(CHROMA_STORE_FILENAME).is_file() || meta_path.is_file();
    if !exists {
        return health(false, 0, "collection_missing");
    }
    let Some(meta) = meta else {
        return health(false, 0, "metadata_missing");
    };
    let document_count = meta
        .get("document_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let indexed_fingerprint = meta.get("source_cache_fingerprint").and_then(Value::as_str);
    let source = crate::issue_cache::issue_cache_source_fingerprint(issue_cache_root)
        .ok()
        .flatten();
    let mut reason = "ready";
    if let Some(source) = &source {
        if source.document_count > 0 && document_count != source.document_count as u64 {
            reason = "count_mismatch";
        } else if source.document_count > 0
            && indexed_fingerprint != Some(source.fingerprint.as_str())
        {
            reason = "source_cache_changed";
        }
    }
    let mut value = health(reason == "ready", document_count, reason);
    value["exists"] = Value::Bool(true);
    value["last_repair_at"] = meta
        .get("last_full_refresh")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(ttl) = meta.get("ttl_minutes") {
        value["ttl_minutes"] = ttl.clone();
    }
    if let Some(fingerprint) = indexed_fingerprint {
        value["source_cache_fingerprint"] = Value::String(fingerprint.to_string());
    }
    if reason != "ready" {
        if let Some(source) = &source {
            value["current_source_cache_fingerprint"] = Value::String(source.fingerprint.clone());
            value["current_source_document_count"] = json!(source.document_count);
        }
    }
    value
}

fn repo_scope_status(repo_dir: &Path, scope: &str) -> Value {
    let db_path = repo_dir.join(scope);
    let manifest_path = repo_dir.join(format!("manifest-{scope}.json"));
    scope_store_status(&db_path, &manifest_path, false)
}

fn legacy_worktree_scope_status(repo_dir: &Path, worktree_hash: &str, scope: &str) -> Value {
    let worktree_dir = repo_dir.join("worktrees").join(worktree_hash);
    let db_path = worktree_dir.join(scope);
    let manifest_path = worktree_dir.join(format!("manifest-{scope}.json"));
    let legacy_residue =
        worktree_dir.join("specs").exists() || worktree_dir.join("manifest-specs.json").exists();
    let mut value = scope_store_status(&db_path, &manifest_path, legacy_residue);
    value["legacy_residue_detected"] = Value::Bool(legacy_residue);
    value
}

fn scope_store_status(db_path: &Path, manifest_path: &Path, legacy_residue: bool) -> Value {
    if active_pointer_corrupt(db_path) {
        return health(false, 0, "active_pointer_corrupt");
    }
    let exists = active_store(db_path).join(CHROMA_STORE_FILENAME).is_file();
    if !exists {
        return health(false, 0, "collection_missing");
    }
    let Some(document_count) = manifest_entry_count(manifest_path) else {
        let mut value = health(false, 0, "manifest_missing");
        value["exists"] = Value::Bool(true);
        return value;
    };
    if legacy_residue {
        let mut value = health(false, document_count, "legacy_residue");
        value["exists"] = Value::Bool(true);
        return value;
    }
    health(true, document_count, "ready")
}

// ---------------------------------------------------------------------------
// Additive v2 Worktree View selection (Phase 71 data-model "Reader Selection")
// ---------------------------------------------------------------------------

struct V2View {
    view_id: String,
    visible_counts: BTreeMap<String, u64>,
}

struct V2Selection {
    view: Option<V2View>,
    reason: String,
    repair_required: bool,
    fallback_source: Option<&'static str>,
}

fn missing_selection(reason: &str) -> V2Selection {
    V2Selection {
        view: None,
        reason: reason.to_string(),
        repair_required: true,
        fallback_source: None,
    }
}

/// `(exists, parsed head)` so a present-but-invalid head is reported as
/// `view_head_invalid` instead of `view_head_missing`.
fn read_head(path: &Path) -> (bool, Option<WorktreeViewHead>) {
    match fs::read(path) {
        Ok(bytes) => (true, serde_json::from_slice(&bytes).ok()),
        Err(_) => (false, None),
    }
}

fn select_v2_view(v2_root: &Path, repo_hash: &str, worktree_hash: &str) -> V2Selection {
    let worktree_root = v2_root.join("worktrees").join(worktree_hash);
    let (head_exists, head) = read_head(&worktree_root.join("head.json"));
    let mut primary_issue: Option<&'static str> = None;
    let journal = if head.is_none() {
        if head_exists {
            primary_issue = Some("view_head_invalid");
        }
        read_head(&worktree_root.join("head.previous.json")).1
    } else {
        None
    };
    let Some(source_head) = head.as_ref().or(journal.as_ref()) else {
        return missing_selection(primary_issue.unwrap_or("view_head_missing"));
    };

    let mut candidates = vec![(source_head.active_view_id.as_str(), "active")];
    if let Some(previous) = source_head.previous_view_id.as_deref() {
        candidates.push((previous, "previous"));
    }
    for (view_id, role) in candidates {
        match inspect_v2_view(v2_root, &worktree_root, repo_hash, worktree_hash, view_id) {
            Ok(view) => {
                let fallback = head.is_none() || role == "previous";
                return V2Selection {
                    view: Some(view),
                    reason: if fallback {
                        primary_issue.unwrap_or("view_closure_invalid").to_string()
                    } else {
                        "ready".to_string()
                    },
                    repair_required: fallback,
                    fallback_source: fallback.then_some("previous"),
                };
            }
            Err(reason) => {
                if role == "active" && primary_issue.is_none() {
                    primary_issue = Some(reason);
                }
            }
        }
    }
    missing_selection(primary_issue.unwrap_or("view_closure_invalid"))
}

fn semantic_compatibility(compatibility: &FileIndexCompatibilityDescriptor) -> Option<Value> {
    let mut value = serde_json::to_value(compatibility).ok()?;
    value.as_object_mut()?.remove("runner_hash");
    Some(value)
}

struct VerifiedManifest {
    records: Vec<Value>,
    paths_by_scope: BTreeMap<&'static str, BTreeSet<String>>,
}

/// Structural verification of one immutable generation manifest: schema,
/// canonical digest, sorted unique paths, and per-scope path sets.
fn verify_manifest(artifact_dir: &Path, expected_digest: &str) -> Option<VerifiedManifest> {
    let payload = read_json(&artifact_dir.join("manifest.json"))?;
    let object = payload.as_object()?;
    if object.len() != 2 || object.get("schema_version")?.as_u64()? != 1 {
        return None;
    }
    let entries = object.get("entries")?.as_array()?;
    if canonical_json_sha256(&Value::Array(entries.clone())) != expected_digest {
        return None;
    }
    let mut records = Vec::with_capacity(entries.len());
    let mut paths_by_scope: BTreeMap<&'static str, BTreeSet<String>> = FILE_SCOPES
        .into_iter()
        .map(|scope| (scope, BTreeSet::new()))
        .collect();
    let mut previous_path: Option<&str> = None;
    for entry in entries {
        let path = entry.get("path")?.as_str()?;
        let cas_key = entry.get("cas_key")?.as_str()?;
        let scope = entry.get("scope")?.as_str()?;
        if previous_path.is_some_and(|previous| previous >= path) {
            return None;
        }
        previous_path = Some(path);
        let scope_key = FILE_SCOPES.into_iter().find(|known| *known == scope)?;
        paths_by_scope.get_mut(scope_key)?.insert(path.to_string());
        records.push(json!([path, cas_key]));
    }
    Some(VerifiedManifest {
        records,
        paths_by_scope,
    })
}

fn inspect_v2_view(
    v2_root: &Path,
    worktree_root: &Path,
    repo_hash: &str,
    worktree_hash: &str,
    view_id: &str,
) -> Result<V2View, &'static str> {
    let view_dir = worktree_root.join("views").join(view_id);
    let descriptor: WorktreeViewDescriptor = read_json(&view_dir.join("descriptor.json"))
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or("view_descriptor_invalid")?;
    if descriptor.view_id != view_id
        || descriptor.repo_hash != repo_hash
        || descriptor.worktree_hash != worktree_hash
    {
        return Err("view_descriptor_invalid");
    }
    let base_dir = v2_root.join("bases").join(&descriptor.base_generation_id);
    let overlay_dir = worktree_root
        .join("overlays")
        .join(&descriptor.overlay_generation_id);
    let closure = verify_closure(&descriptor, &base_dir, &overlay_dir);
    if !closure {
        return Err("view_closure_invalid");
    }
    Ok(V2View {
        view_id: descriptor.view_id,
        visible_counts: descriptor
            .visible_counts
            .into_iter()
            .map(|(scope, count)| (scope, count as u64))
            .collect(),
    })
}

fn verify_closure(
    descriptor: &WorktreeViewDescriptor,
    base_dir: &Path,
    overlay_dir: &Path,
) -> bool {
    let Some(base): Option<BaseGenerationDescriptor> =
        read_json(&base_dir.join("descriptor.json")).and_then(|v| serde_json::from_value(v).ok())
    else {
        return false;
    };
    let Some(overlay): Option<OverlayGenerationDescriptor> =
        read_json(&overlay_dir.join("descriptor.json"))
            .and_then(|v| serde_json::from_value(v).ok())
    else {
        return false;
    };
    let Some(view_compatibility) = semantic_compatibility(&descriptor.compatibility) else {
        return false;
    };
    if base.base_generation_id != descriptor.base_generation_id
        || base.repo_hash != descriptor.repo_hash
        || overlay.overlay_generation_id != descriptor.overlay_generation_id
        || overlay.repo_hash != descriptor.repo_hash
        || overlay.worktree_hash != descriptor.worktree_hash
        || overlay.base_generation_id != descriptor.base_generation_id
        || overlay.source_snapshot_id != descriptor.source_snapshot_id
        || !base
            .compatibility
            .is_semantically_compatible_with(&descriptor.compatibility)
        || !overlay
            .compatibility
            .is_semantically_compatible_with(&descriptor.compatibility)
    {
        return false;
    }
    let Some(base_manifest) = verify_manifest(base_dir, &base.manifest_digest) else {
        return false;
    };
    let Some(overlay_manifest) = verify_manifest(overlay_dir, &overlay.manifest_digest) else {
        return false;
    };
    let base_identity = json!({
        "compatibility": view_compatibility,
        "records": base_manifest.records,
        "root_tree_oid": base.root_tree_oid,
    });
    let overlay_identity = json!({
        "compatibility": view_compatibility,
        "records": overlay_manifest.records,
        "source_snapshot_id": overlay.source_snapshot_id,
        "base_generation_id": overlay.base_generation_id,
    });
    if canonical_json_sha256(&base_identity) != base.base_generation_id
        || canonical_json_sha256(&overlay_identity) != overlay.overlay_generation_id
    {
        return false;
    }
    let base_files = &base_manifest.paths_by_scope["files"];
    let base_docs = &base_manifest.paths_by_scope["files-docs"];
    if base.document_counts.files != base_files.len()
        || base.document_counts.files_docs != base_docs.len()
    {
        return false;
    }
    let overlay_files = &overlay_manifest.paths_by_scope["files"];
    let overlay_docs = &overlay_manifest.paths_by_scope["files-docs"];
    let tombstones: BTreeSet<String> = overlay.tombstones.iter().cloned().collect();
    let overlay_all: BTreeSet<&String> = overlay_files.iter().chain(overlay_docs.iter()).collect();
    let base_all: BTreeSet<&String> = base_files.iter().chain(base_docs.iter()).collect();
    if tombstones.iter().any(|path| overlay_all.contains(path))
        || !tombstones.iter().all(|path| base_all.contains(path))
    {
        return false;
    }
    let changed_or_removed =
        |path: &String| overlay_all.contains(path) || tombstones.contains(path);
    let expected_files_shadow: BTreeSet<String> = overlay_files
        .iter()
        .cloned()
        .chain(
            base_files
                .iter()
                .filter(|path| changed_or_removed(path))
                .cloned(),
        )
        .collect();
    let expected_docs_shadow: BTreeSet<String> = overlay_docs
        .iter()
        .cloned()
        .chain(
            base_docs
                .iter()
                .filter(|path| changed_or_removed(path))
                .cloned(),
        )
        .collect();
    let files_shadow: BTreeSet<String> = overlay.files_shadow.iter().cloned().collect();
    let docs_shadow: BTreeSet<String> = overlay.files_docs_shadow.iter().cloned().collect();
    if files_shadow != expected_files_shadow || docs_shadow != expected_docs_shadow {
        return false;
    }
    let expected_files = base_files.difference(&files_shadow).count() + overlay_files.len();
    let expected_docs = base_docs.difference(&docs_shadow).count() + overlay_docs.len();
    descriptor.visible_counts.get("files").copied() == Some(expected_files)
        && descriptor.visible_counts.get("files-docs").copied() == Some(expected_docs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_index_projects_repair_required_for_every_scope() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let status = repo_shared_status(tmp.path(), "repo", &tmp.path().join("cache"));
        for scope in ["issues", "specs", "memory", "discussions", "board"] {
            let value = &status[scope];
            assert_eq!(value["healthy"], Value::Bool(false), "{scope}");
            assert_eq!(value["repair_required"], Value::Bool(true), "{scope}");
            assert_eq!(value["reason"], "collection_missing", "{scope}");
        }
        let files = worktree_file_status(tmp.path(), "repo", "wt");
        for scope in FILE_SCOPES {
            assert_eq!(files[scope]["reason"], "view_head_missing", "{scope}");
            assert_eq!(files[scope]["healthy"], Value::Bool(false), "{scope}");
            assert!(files[scope].get("view_id").is_none(), "{scope}");
        }
        assert!(
            !tmp.path().join("repo").exists(),
            "projection must not create index directories"
        );
    }

    #[test]
    fn ready_repo_scope_counts_manifest_entries_without_a_runner() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_dir = tmp.path().join("repo");
        fs::create_dir_all(repo_dir.join("specs")).expect("specs dir");
        fs::write(repo_dir.join("specs").join(CHROMA_STORE_FILENAME), b"db").expect("store");
        fs::write(
            repo_dir.join("manifest-specs.json"),
            r#"{"schema_version":1,"scope":"specs","entries":[{"path":"a","mtime":1,"size":1},{"path":"b","mtime":1,"size":1}]}"#,
        )
        .expect("manifest");
        let status = repo_shared_status(tmp.path(), "repo", &tmp.path().join("cache"));
        assert_eq!(status["specs"]["healthy"], Value::Bool(true));
        assert_eq!(status["specs"]["document_count"], json!(2));
        assert_eq!(status["specs"]["reason"], "ready");
    }

    #[test]
    fn corrupt_active_pointer_is_reported_before_store_presence() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_dir = tmp.path().join("repo");
        fs::create_dir_all(repo_dir.join("memory.gen")).expect("gen dir");
        fs::write(
            repo_dir.join("memory.gen").join(ACTIVE_POINTER_FILENAME),
            b"{broken",
        )
        .expect("pointer");
        let status = repo_shared_status(tmp.path(), "repo", &tmp.path().join("cache"));
        assert_eq!(status["memory"]["reason"], "active_pointer_corrupt");
    }

    #[test]
    fn invalid_head_bytes_project_view_head_invalid() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let worktree_root = tmp
            .path()
            .join("repo")
            .join("file-index-v2")
            .join("worktrees")
            .join("wt");
        fs::create_dir_all(&worktree_root).expect("worktree root");
        fs::write(worktree_root.join("head.json"), b"{not-json").expect("head");
        let files = worktree_file_status(tmp.path(), "repo", "wt");
        assert_eq!(files["files"]["reason"], "view_head_invalid");
        assert_eq!(files["files-docs"]["reason"], "view_head_invalid");
        assert_eq!(files["files"]["healthy"], Value::Bool(false));
    }
}
