//! Additive Phase 71 file-index descriptor schemas.
//!
//! These data-only types define the Rust/Python boundary. Publication and
//! reader fallback are intentionally implemented in later TDD slices; adding
//! the schema here does not make v2 artifacts visible to legacy readers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDocumentContract {
    pub payload_builder_version: u32,
    pub decode: String,
    pub content_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndexCompatibilityDescriptor {
    pub layout_version: u32,
    pub index_schema_version: u32,
    pub scope_set: Vec<String>,
    pub model_id: String,
    pub model_revision: String,
    pub dimension: usize,
    pub normalization: String,
    pub metric: String,
    pub query_prefix: String,
    pub passage_prefix: String,
    pub document_contract: FileDocumentContract,
    pub path_policy_hash: String,
    pub writer_protocol: String,
    /// Diagnostic provenance. It is deliberately excluded from semantic
    /// compatibility so a byte-identical writer rebuild does not invalidate
    /// otherwise safe immutable artifacts.
    pub runner_hash: String,
}

impl FileIndexCompatibilityDescriptor {
    pub fn is_semantically_compatible_with(&self, other: &Self) -> bool {
        self.layout_version == other.layout_version
            && self.index_schema_version == other.index_schema_version
            && self.scope_set == other.scope_set
            && self.model_id == other.model_id
            && self.model_revision == other.model_revision
            && self.dimension == other.dimension
            && self.normalization == other.normalization
            && self.metric == other.metric
            && self.query_prefix == other.query_prefix
            && self.passage_prefix == other.passage_prefix
            && self.document_contract == other.document_contract
            && self.path_policy_hash == other.path_policy_hash
            && self.writer_protocol == other.writer_protocol
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileGenerationReference {
    pub store: String,
    pub collection: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndexDocumentCounts {
    pub files: usize,
    #[serde(rename = "files-docs")]
    pub files_docs: usize,
    pub total: usize,
}

/// Immutable repo-scoped aggregate containing both Files collections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseGenerationDescriptor {
    pub schema_version: u32,
    pub kind: String,
    pub base_generation_id: String,
    pub repo_hash: String,
    pub root_tree_oid: Option<String>,
    pub canonical_ref: Option<String>,
    pub compatibility: FileIndexCompatibilityDescriptor,
    pub files_generation: FileGenerationReference,
    pub files_docs_generation: FileGenerationReference,
    pub manifest_digest: String,
    pub document_counts: FileIndexDocumentCounts,
    pub build_state: String,
    pub created_at: String,
    pub verified_at: String,
}

/// Immutable worktree-scoped aggregate containing both overlay collections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayGenerationDescriptor {
    pub schema_version: u32,
    pub kind: String,
    pub overlay_generation_id: String,
    pub repo_hash: String,
    pub worktree_hash: String,
    pub base_generation_id: String,
    pub source_snapshot_id: String,
    pub compatibility: FileIndexCompatibilityDescriptor,
    pub files_generation: FileGenerationReference,
    pub files_docs_generation: FileGenerationReference,
    pub files_shadow: Vec<String>,
    pub files_docs_shadow: Vec<String>,
    pub tombstones: Vec<String>,
    pub manifest_digest: String,
    pub build_state: String,
    pub created_at: String,
    pub verified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeViewDescriptor {
    pub schema_version: u32,
    pub view_id: String,
    pub repo_hash: String,
    pub worktree_hash: String,
    pub base_generation_id: String,
    pub overlay_generation_id: String,
    pub compatibility: FileIndexCompatibilityDescriptor,
    pub visible_counts: BTreeMap<String, usize>,
    pub source_snapshot_id: String,
    pub descriptor_checksum: String,
    pub verified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeViewHead {
    pub schema_version: u32,
    pub active_view_id: String,
    pub previous_view_id: Option<String>,
    pub sequence: u64,
    pub checksum: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compatibility() -> FileIndexCompatibilityDescriptor {
        FileIndexCompatibilityDescriptor {
            layout_version: 2,
            index_schema_version: 1,
            scope_set: vec!["files".into(), "files-docs".into()],
            model_id: "intfloat/multilingual-e5-base".into(),
            model_revision: "d128750597153bb5987e10b1c3493a34e5a4502a".into(),
            dimension: 768,
            normalization: "none".into(),
            metric: "cosine".into(),
            query_prefix: "query: ".into(),
            passage_prefix: "passage: ".into(),
            document_contract: FileDocumentContract {
                payload_builder_version: 1,
                decode: "utf-8-replace".into(),
                content_limit: 2000,
            },
            path_policy_hash: "policy".into(),
            writer_protocol: "file-index-v2".into(),
            runner_hash: "runner".into(),
        }
    }

    #[test]
    fn aggregate_generation_descriptors_roundtrip_python_json_shape() {
        let compatibility = serde_json::to_value(compatibility()).unwrap();
        let base_json = serde_json::json!({
            "schema_version": 1,
            "kind": "base",
            "base_generation_id": "base",
            "repo_hash": "repo",
            "root_tree_oid": "tree",
            "canonical_ref": "origin/develop",
            "compatibility": compatibility.clone(),
            "files_generation": {"store": "store", "collection": "files_code"},
            "files_docs_generation": {"store": "store", "collection": "files_docs"},
            "manifest_digest": "manifest",
            "document_counts": {"files": 8, "files-docs": 2, "total": 10},
            "build_state": "verified",
            "created_at": "2026-08-29T00:00:00+00:00",
            "verified_at": "2026-08-29T00:00:00+00:00"
        });
        let base: BaseGenerationDescriptor = serde_json::from_value(base_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(base).unwrap(), base_json);

        let overlay_json = serde_json::json!({
            "schema_version": 1,
            "kind": "overlay",
            "overlay_generation_id": "overlay",
            "repo_hash": "repo",
            "worktree_hash": "worktree",
            "base_generation_id": "base",
            "source_snapshot_id": "snapshot",
            "compatibility": compatibility,
            "files_generation": {"store": "store", "collection": "files_code"},
            "files_docs_generation": {"store": "store", "collection": "files_docs"},
            "files_shadow": ["src/lib.rs"],
            "files_docs_shadow": ["README.md"],
            "tombstones": ["deleted.rs"],
            "manifest_digest": "manifest",
            "build_state": "verified",
            "created_at": "2026-08-29T00:00:00+00:00",
            "verified_at": "2026-08-29T00:00:00+00:00"
        });
        let overlay: OverlayGenerationDescriptor =
            serde_json::from_value(overlay_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(overlay).unwrap(), overlay_json);
    }
}
