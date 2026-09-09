//! Additive Phase 71 file-index descriptor schemas.
//!
//! These data-only types define the Rust/Python boundary. Publication and
//! reader fallback are intentionally implemented in later TDD slices; adding
//! the schema here does not make v2 artifacts visible to legacy readers.

use std::collections::BTreeMap;

use chrono::DateTime;
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const FILE_INDEX_SCHEMA_VERSION: u32 = 1;

fn is_safe_artifact_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_alphanumeric())
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value == FILE_INDEX_SCHEMA_VERSION {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "unsupported file-index descriptor schema version",
        ))
    }
}

fn deserialize_positive_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value > 0 {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "file-index descriptor value must be positive",
        ))
    }
}

fn deserialize_layout_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value == 2 {
        Ok(value)
    } else {
        Err(D::Error::custom("unsupported file-index layout version"))
    }
}

fn deserialize_index_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_schema_version(deserializer)
}

fn deserialize_positive_usize<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    if value > 0 {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "file-index descriptor value must be positive",
        ))
    }
}

fn deserialize_nonempty_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        Err(D::Error::custom(
            "file-index descriptor string must not be empty",
        ))
    } else {
        Ok(value)
    }
}

fn deserialize_timestamp<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = deserialize_nonempty_string(deserializer)?;
    DateTime::parse_from_rfc3339(&value)
        .map(|_| value)
        .map_err(|_| D::Error::custom("file-index timestamp must be timezone-aware ISO-8601"))
}

fn is_safe_logical_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .chars()
                    .all(|character| character > '\u{1f}' && character != '\u{7f}')
        })
}

fn deserialize_sorted_unique_paths<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Vec::<String>::deserialize(deserializer)?;
    if value.iter().all(|path| is_safe_logical_path(path))
        && value.windows(2).all(|pair| pair[0] < pair[1])
    {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "file-index path lists must be sorted, unique, and safe",
        ))
    }
}

fn write_canonical_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character <= '\u{1f}' || !character.is_ascii() => {
                let codepoint = character as u32;
                if codepoint <= 0xffff {
                    output.push_str(&format!("\\u{codepoint:04x}"));
                } else {
                    let supplementary = codepoint - 0x1_0000;
                    let high = 0xd800 + (supplementary >> 10);
                    let low = 0xdc00 + (supplementary & 0x3ff);
                    output.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn write_canonical_json(output: &mut String, value: &Value) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => write_canonical_json_string(output, value),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json(output, value);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical_json_string(output, key);
                output.push(':');
                write_canonical_json(output, value);
            }
            output.push('}');
        }
    }
}

/// SHA-256 of the canonical JSON encoding shared with the Python runner
/// (`sort_keys`, compact separators, ASCII-escaped strings).
pub fn canonical_json_sha256(value: &Value) -> String {
    let mut canonical = String::new();
    write_canonical_json(&mut canonical, value);
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

fn deserialize_optional_nonempty_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if value.as_deref() == Some("") {
        Err(D::Error::custom(
            "file-index descriptor string must not be empty",
        ))
    } else {
        Ok(value)
    }
}

fn deserialize_safe_artifact_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if is_safe_artifact_id(&value) {
        Ok(value)
    } else {
        Err(D::Error::custom("unsafe file-index artifact id"))
    }
}

fn deserialize_optional_safe_artifact_id<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if value.as_deref().is_none_or(is_safe_artifact_id) {
        Ok(value)
    } else {
        Err(D::Error::custom("unsafe file-index artifact id"))
    }
}

fn deserialize_sha256<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if is_lowercase_sha256(&value) {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "file-index checksum must be lowercase SHA-256",
        ))
    }
}

fn deserialize_exact_string<'de, D>(deserializer: D, expected: &str) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == expected {
        Ok(value)
    } else {
        Err(D::Error::custom(format!("expected {expected:?}")))
    }
}

fn deserialize_base_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "base")
}

fn deserialize_overlay_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "overlay")
}

fn deserialize_verified_state<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "verified")
}

fn deserialize_writer_protocol<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "file-index-v2")
}

fn deserialize_scope_set<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Vec::<String>::deserialize(deserializer)?;
    if value == ["files", "files-docs"] {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "file-index scope_set must contain both file scopes",
        ))
    }
}

fn is_safe_relative_artifact_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && value.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && component.len() <= 160
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

/// Reason an immutable file-index artifact is temporarily rooted for GC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileIndexGcPinKind {
    Reader,
    Migration,
    Continuation,
}

/// Cross-language descriptor for one live GC root.
///
/// Liveness is established by the sibling kernel-locked `.lock` file. This
/// JSON is deliberately only the strictly validated description of which
/// relative artifact roots that live owner protects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedFileIndexGcPinDescriptor")]
pub struct FileIndexGcPinDescriptor {
    pub schema_version: u32,
    pub pin_id: String,
    pub kind: FileIndexGcPinKind,
    pub repo_hash: String,
    pub worktree_hash: Option<String>,
    pub protected_paths: Vec<String>,
    pub owner_pid: u32,
    pub created_at: String,
    pub checksum: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedFileIndexGcPinDescriptor {
    schema_version: u32,
    pin_id: String,
    kind: FileIndexGcPinKind,
    repo_hash: String,
    worktree_hash: Option<String>,
    protected_paths: Vec<String>,
    owner_pid: u32,
    created_at: String,
    checksum: String,
}

impl FileIndexGcPinDescriptor {
    pub(crate) fn new(
        pin_id: String,
        kind: FileIndexGcPinKind,
        repo_hash: String,
        worktree_hash: Option<String>,
        protected_paths: Vec<String>,
        owner_pid: u32,
        created_at: String,
    ) -> Result<Self, String> {
        let mut payload = serde_json::json!({
            "schema_version": FILE_INDEX_SCHEMA_VERSION,
            "pin_id": pin_id,
            "kind": kind,
            "repo_hash": repo_hash,
            "worktree_hash": worktree_hash,
            "protected_paths": protected_paths,
            "owner_pid": owner_pid,
            "created_at": created_at,
        });
        let checksum = canonical_json_sha256(&payload);
        payload
            .as_object_mut()
            .expect("file-index GC pin serialization is an object")
            .insert("checksum".to_string(), Value::String(checksum));
        serde_json::from_value(payload).map_err(|error| error.to_string())
    }
}

impl TryFrom<UncheckedFileIndexGcPinDescriptor> for FileIndexGcPinDescriptor {
    type Error = String;

    fn try_from(raw: UncheckedFileIndexGcPinDescriptor) -> Result<Self, Self::Error> {
        if raw.schema_version != FILE_INDEX_SCHEMA_VERSION
            || !is_safe_artifact_id(&raw.pin_id)
            || !is_safe_artifact_id(&raw.repo_hash)
            || raw
                .worktree_hash
                .as_deref()
                .is_some_and(|value| !is_safe_artifact_id(value))
            || raw.protected_paths.is_empty()
            || !raw
                .protected_paths
                .iter()
                .all(|path| is_safe_relative_artifact_path(path))
            || raw.owner_pid == 0
            || DateTime::parse_from_rfc3339(&raw.created_at).is_err()
            || !is_lowercase_sha256(&raw.checksum)
        {
            return Err("invalid file-index GC pin descriptor".to_string());
        }

        let value = Self {
            schema_version: raw.schema_version,
            pin_id: raw.pin_id,
            kind: raw.kind,
            repo_hash: raw.repo_hash,
            worktree_hash: raw.worktree_hash,
            protected_paths: raw.protected_paths,
            owner_pid: raw.owner_pid,
            created_at: raw.created_at,
            checksum: raw.checksum,
        };
        let mut checksum_payload =
            serde_json::to_value(&value).map_err(|error| error.to_string())?;
        checksum_payload
            .as_object_mut()
            .expect("file-index GC pin serialization is an object")
            .remove("checksum");
        if canonical_json_sha256(&checksum_payload) != value.checksum {
            return Err("invalid file-index GC pin descriptor checksum".to_string());
        }
        Ok(value)
    }
}

fn deserialize_document_counts<'de, D>(deserializer: D) -> Result<FileIndexDocumentCounts, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = FileIndexDocumentCounts::deserialize(deserializer)?;
    if value.files.checked_add(value.files_docs) == Some(value.total) {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "file-index total count must equal both scope counts",
        ))
    }
}

fn deserialize_generation_reference<'de, D>(
    deserializer: D,
    expected_collection: &str,
) -> Result<FileGenerationReference, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = FileGenerationReference::deserialize(deserializer)?;
    if value.store == "store" && value.collection == expected_collection {
        Ok(value)
    } else {
        Err(D::Error::custom("invalid file-index generation reference"))
    }
}

fn deserialize_files_generation<'de, D>(
    deserializer: D,
) -> Result<FileGenerationReference, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_generation_reference(deserializer, "files_code")
}

fn deserialize_files_docs_generation<'de, D>(
    deserializer: D,
) -> Result<FileGenerationReference, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_generation_reference(deserializer, "files_docs")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDocumentContract {
    #[serde(deserialize_with = "deserialize_positive_u32")]
    pub payload_builder_version: u32,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub decode: String,
    #[serde(deserialize_with = "deserialize_positive_usize")]
    pub content_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIndexCompatibilityDescriptor {
    #[serde(deserialize_with = "deserialize_layout_version")]
    pub layout_version: u32,
    #[serde(deserialize_with = "deserialize_index_schema_version")]
    pub index_schema_version: u32,
    #[serde(deserialize_with = "deserialize_scope_set")]
    pub scope_set: Vec<String>,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub model_id: String,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub model_revision: String,
    #[serde(deserialize_with = "deserialize_positive_usize")]
    pub dimension: usize,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub normalization: String,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub metric: String,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub query_prefix: String,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub passage_prefix: String,
    pub document_contract: FileDocumentContract,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub path_policy_hash: String,
    #[serde(deserialize_with = "deserialize_writer_protocol")]
    pub writer_protocol: String,
    /// Diagnostic provenance. It is deliberately excluded from semantic
    /// compatibility so a byte-identical writer rebuild does not invalidate
    /// otherwise safe immutable artifacts.
    #[serde(deserialize_with = "deserialize_nonempty_string")]
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
#[serde(deny_unknown_fields)]
pub struct FileGenerationReference {
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub store: String,
    #[serde(deserialize_with = "deserialize_nonempty_string")]
    pub collection: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIndexDocumentCounts {
    pub files: usize,
    #[serde(rename = "files-docs")]
    pub files_docs: usize,
    pub total: usize,
}

/// Immutable repo-scoped aggregate containing both Files collections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaseGenerationDescriptor {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    #[serde(deserialize_with = "deserialize_base_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_safe_artifact_id")]
    pub base_generation_id: String,
    #[serde(deserialize_with = "deserialize_safe_artifact_id")]
    pub repo_hash: String,
    #[serde(deserialize_with = "deserialize_optional_safe_artifact_id")]
    pub root_tree_oid: Option<String>,
    #[serde(deserialize_with = "deserialize_optional_nonempty_string")]
    pub canonical_ref: Option<String>,
    pub compatibility: FileIndexCompatibilityDescriptor,
    #[serde(deserialize_with = "deserialize_files_generation")]
    pub files_generation: FileGenerationReference,
    #[serde(deserialize_with = "deserialize_files_docs_generation")]
    pub files_docs_generation: FileGenerationReference,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub manifest_digest: String,
    #[serde(deserialize_with = "deserialize_document_counts")]
    pub document_counts: FileIndexDocumentCounts,
    #[serde(deserialize_with = "deserialize_verified_state")]
    pub build_state: String,
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub created_at: String,
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub verified_at: String,
}

/// Immutable worktree-scoped aggregate containing both overlay collections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayGenerationDescriptor {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    #[serde(deserialize_with = "deserialize_overlay_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_safe_artifact_id")]
    pub overlay_generation_id: String,
    #[serde(deserialize_with = "deserialize_safe_artifact_id")]
    pub repo_hash: String,
    #[serde(deserialize_with = "deserialize_safe_artifact_id")]
    pub worktree_hash: String,
    #[serde(deserialize_with = "deserialize_safe_artifact_id")]
    pub base_generation_id: String,
    #[serde(deserialize_with = "deserialize_safe_artifact_id")]
    pub source_snapshot_id: String,
    pub compatibility: FileIndexCompatibilityDescriptor,
    #[serde(deserialize_with = "deserialize_files_generation")]
    pub files_generation: FileGenerationReference,
    #[serde(deserialize_with = "deserialize_files_docs_generation")]
    pub files_docs_generation: FileGenerationReference,
    #[serde(deserialize_with = "deserialize_sorted_unique_paths")]
    pub files_shadow: Vec<String>,
    #[serde(deserialize_with = "deserialize_sorted_unique_paths")]
    pub files_docs_shadow: Vec<String>,
    #[serde(deserialize_with = "deserialize_sorted_unique_paths")]
    pub tombstones: Vec<String>,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub manifest_digest: String,
    #[serde(deserialize_with = "deserialize_verified_state")]
    pub build_state: String,
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub created_at: String,
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub verified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedWorktreeViewDescriptor")]
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
#[serde(try_from = "UncheckedWorktreeViewHead")]
pub struct WorktreeViewHead {
    pub schema_version: u32,
    pub active_view_id: String,
    pub previous_view_id: Option<String>,
    pub sequence: u64,
    pub checksum: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedWorktreeViewDescriptor {
    schema_version: u32,
    view_id: String,
    repo_hash: String,
    worktree_hash: String,
    base_generation_id: String,
    overlay_generation_id: String,
    compatibility: FileIndexCompatibilityDescriptor,
    visible_counts: BTreeMap<String, usize>,
    source_snapshot_id: String,
    descriptor_checksum: String,
    verified_at: String,
}

impl TryFrom<UncheckedWorktreeViewDescriptor> for WorktreeViewDescriptor {
    type Error = String;

    fn try_from(raw: UncheckedWorktreeViewDescriptor) -> Result<Self, Self::Error> {
        if raw.schema_version != FILE_INDEX_SCHEMA_VERSION
            || ![
                raw.view_id.as_str(),
                raw.repo_hash.as_str(),
                raw.worktree_hash.as_str(),
                raw.base_generation_id.as_str(),
                raw.overlay_generation_id.as_str(),
                raw.source_snapshot_id.as_str(),
            ]
            .into_iter()
            .all(is_safe_artifact_id)
            || !raw
                .visible_counts
                .keys()
                .map(String::as_str)
                .eq(["files", "files-docs"])
            || !is_lowercase_sha256(&raw.descriptor_checksum)
            || DateTime::parse_from_rfc3339(&raw.verified_at).is_err()
        {
            return Err("invalid file-index WorktreeView descriptor".to_string());
        }
        let value = Self {
            schema_version: raw.schema_version,
            view_id: raw.view_id,
            repo_hash: raw.repo_hash,
            worktree_hash: raw.worktree_hash,
            base_generation_id: raw.base_generation_id,
            overlay_generation_id: raw.overlay_generation_id,
            compatibility: raw.compatibility,
            visible_counts: raw.visible_counts,
            source_snapshot_id: raw.source_snapshot_id,
            descriptor_checksum: raw.descriptor_checksum,
            verified_at: raw.verified_at,
        };
        let mut checksum_payload =
            serde_json::to_value(&value).map_err(|error| error.to_string())?;
        checksum_payload
            .as_object_mut()
            .expect("WorktreeView serialization is an object")
            .remove("descriptor_checksum");
        if canonical_json_sha256(&checksum_payload) != value.descriptor_checksum {
            return Err("invalid file-index WorktreeView descriptor checksum".to_string());
        }
        let mut semantic_compatibility =
            serde_json::to_value(&value.compatibility).map_err(|error| error.to_string())?;
        semantic_compatibility
            .as_object_mut()
            .expect("compatibility serialization is an object")
            .remove("runner_hash");
        let view_identity = serde_json::json!({
            "schema_version": value.schema_version,
            "repo_hash": value.repo_hash,
            "worktree_hash": value.worktree_hash,
            "base_generation_id": value.base_generation_id,
            "overlay_generation_id": value.overlay_generation_id,
            "compatibility": semantic_compatibility,
            "visible_counts": value.visible_counts,
            "source_snapshot_id": value.source_snapshot_id,
        });
        if canonical_json_sha256(&view_identity) != value.view_id {
            return Err("invalid content-addressed WorktreeView id".to_string());
        }
        Ok(value)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedWorktreeViewHead {
    schema_version: u32,
    active_view_id: String,
    previous_view_id: Option<String>,
    sequence: u64,
    checksum: String,
}

impl TryFrom<UncheckedWorktreeViewHead> for WorktreeViewHead {
    type Error = String;

    fn try_from(raw: UncheckedWorktreeViewHead) -> Result<Self, Self::Error> {
        if raw.schema_version != FILE_INDEX_SCHEMA_VERSION
            || !is_safe_artifact_id(&raw.active_view_id)
            || raw.previous_view_id.as_deref().is_some_and(|previous| {
                !is_safe_artifact_id(previous) || previous == raw.active_view_id
            })
            || raw.sequence == 0
            || !is_lowercase_sha256(&raw.checksum)
        {
            return Err("invalid file-index WorktreeView head".to_string());
        }
        let value = Self {
            schema_version: raw.schema_version,
            active_view_id: raw.active_view_id,
            previous_view_id: raw.previous_view_id,
            sequence: raw.sequence,
            checksum: raw.checksum,
        };
        let mut checksum_payload =
            serde_json::to_value(&value).map_err(|error| error.to_string())?;
        checksum_payload
            .as_object_mut()
            .expect("WorktreeView head serialization is an object")
            .remove("checksum");
        if canonical_json_sha256(&checksum_payload) != value.checksum {
            return Err("invalid file-index WorktreeView head checksum".to_string());
        }
        Ok(value)
    }
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

    fn signed_view_json(compatibility: Value) -> Value {
        let mut semantic = compatibility.clone();
        semantic.as_object_mut().unwrap().remove("runner_hash");
        let identity = serde_json::json!({
            "schema_version": 1,
            "repo_hash": "repo",
            "worktree_hash": "worktree",
            "base_generation_id": "base",
            "overlay_generation_id": "overlay",
            "compatibility": semantic,
            "visible_counts": {"files": 8, "files-docs": 2},
            "source_snapshot_id": "snapshot",
        });
        let view_id = canonical_json_sha256(&identity);
        let mut descriptor = serde_json::json!({
            "schema_version": 1,
            "view_id": view_id,
            "repo_hash": "repo",
            "worktree_hash": "worktree",
            "base_generation_id": "base",
            "overlay_generation_id": "overlay",
            "compatibility": compatibility,
            "visible_counts": {"files": 8, "files-docs": 2},
            "source_snapshot_id": "snapshot",
            "verified_at": "2026-08-29T00:00:00+00:00"
        });
        let checksum = canonical_json_sha256(&descriptor);
        descriptor["descriptor_checksum"] = Value::String(checksum);
        descriptor
    }

    fn signed_head_json(active_view_id: &str, previous_view_id: Option<&str>) -> Value {
        let mut head = serde_json::json!({
            "schema_version": 1,
            "active_view_id": active_view_id,
            "previous_view_id": previous_view_id,
            "sequence": 1,
        });
        let checksum = canonical_json_sha256(&head);
        head["checksum"] = Value::String(checksum);
        head
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
            "manifest_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
            "compatibility": compatibility.clone(),
            "files_generation": {"store": "store", "collection": "files_code"},
            "files_docs_generation": {"store": "store", "collection": "files_docs"},
            "files_shadow": ["src/lib.rs"],
            "files_docs_shadow": ["README.md"],
            "tombstones": ["deleted.rs"],
            "manifest_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "build_state": "verified",
            "created_at": "2026-08-29T00:00:00+00:00",
            "verified_at": "2026-08-29T00:00:00+00:00"
        });
        let overlay: OverlayGenerationDescriptor =
            serde_json::from_value(overlay_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(overlay).unwrap(), overlay_json);

        let view_json = signed_view_json(compatibility);
        assert_eq!(
            view_json["view_id"],
            "65b62f6ef99fcce5e8da6b839d4c9eb3b1c69c2831f3600cea5e7c3b821b3a70"
        );
        assert_eq!(
            view_json["descriptor_checksum"],
            "bd1d5d430283e6f117f3ddfc9ea95ae1cffd10f2b17517626afec3b2c429ff52"
        );
        let view: WorktreeViewDescriptor = serde_json::from_value(view_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(view).unwrap(), view_json);

        let head_json = signed_head_json(view_json["view_id"].as_str().unwrap(), None);
        assert_eq!(
            head_json["checksum"],
            "9bb85ffe9829fc7c48ec4ea0c5c5e39b72da3a68ab46aec7c4e0b951b82bb9eb"
        );
        let head: WorktreeViewHead = serde_json::from_value(head_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(head).unwrap(), head_json);
        assert_eq!(
            canonical_json_sha256(&serde_json::json!({"unicode": "日本😀"})),
            "423bdcf43aa0682065230883b265cce169ec023620f312383bdce7c93e9b710a"
        );
    }

    #[test]
    fn persisted_descriptors_reject_unknown_fields_and_invalid_semantic_fields() {
        let compatibility = serde_json::to_value(compatibility()).unwrap();
        let valid_view = signed_view_json(compatibility.clone());

        let mut unknown = valid_view.clone();
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<WorktreeViewDescriptor>(unknown).is_err());

        let mut wrong_version = valid_view.clone();
        wrong_version["schema_version"] = serde_json::json!(2);
        assert!(serde_json::from_value::<WorktreeViewDescriptor>(wrong_version).is_err());

        let mut unsafe_id = valid_view.clone();
        unsafe_id["view_id"] = serde_json::json!("../view");
        assert!(serde_json::from_value::<WorktreeViewDescriptor>(unsafe_id).is_err());

        let mut wrong_scopes = valid_view;
        wrong_scopes["visible_counts"] = serde_json::json!({"files": 10});
        assert!(serde_json::from_value::<WorktreeViewDescriptor>(wrong_scopes).is_err());

        let mut wrong_checksum = signed_view_json(compatibility.clone());
        wrong_checksum["verified_at"] = serde_json::json!("2026-08-30T00:00:00+00:00");
        assert!(serde_json::from_value::<WorktreeViewDescriptor>(wrong_checksum).is_err());

        let mut invalid_head = signed_head_json("view-a", None);
        invalid_head["checksum"] = serde_json::json!("not-a-sha256");
        assert!(serde_json::from_value::<WorktreeViewHead>(invalid_head).is_err());

        let same_previous = signed_head_json("view-a", Some("view-a"));
        assert!(serde_json::from_value::<WorktreeViewHead>(same_previous).is_err());

        let mut invalid_base = serde_json::json!({
            "schema_version": 1,
            "kind": "overlay",
            "base_generation_id": "base",
            "repo_hash": "repo",
            "root_tree_oid": "tree",
            "canonical_ref": "origin/develop",
            "compatibility": compatibility,
            "files_generation": {"store": "store", "collection": "files_code"},
            "files_docs_generation": {"store": "store", "collection": "files_docs"},
            "manifest_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "document_counts": {"files": 8, "files-docs": 2, "total": 10},
            "build_state": "verified",
            "created_at": "2026-08-29T00:00:00+00:00",
            "verified_at": "2026-08-29T00:00:00+00:00"
        });
        assert!(serde_json::from_value::<BaseGenerationDescriptor>(invalid_base.clone()).is_err());
        invalid_base["kind"] = serde_json::json!("base");
        invalid_base["build_state"] = serde_json::json!("staged");
        assert!(serde_json::from_value::<BaseGenerationDescriptor>(invalid_base).is_err());

        let mut overflow_counts = serde_json::json!({
            "schema_version": 1,
            "kind": "base",
            "base_generation_id": "base",
            "repo_hash": "repo",
            "root_tree_oid": "tree",
            "canonical_ref": "origin/develop",
            "compatibility": compatibility.clone(),
            "files_generation": {"store": "store", "collection": "files_code"},
            "files_docs_generation": {"store": "store", "collection": "files_docs"},
            "manifest_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "document_counts": {"files": usize::MAX, "files-docs": 1, "total": 0},
            "build_state": "verified",
            "created_at": "2026-08-29T00:00:00+00:00",
            "verified_at": "2026-08-29T00:00:00+00:00"
        });
        assert!(
            serde_json::from_value::<BaseGenerationDescriptor>(overflow_counts.clone()).is_err()
        );
        overflow_counts["document_counts"] =
            serde_json::json!({"files": usize::MAX, "files-docs": 0, "total": usize::MAX});
        assert!(serde_json::from_value::<BaseGenerationDescriptor>(overflow_counts).is_ok());

        let mut invalid_overlay = serde_json::json!({
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
            "files_shadow": ["src/z.rs", "src/a.rs"],
            "files_docs_shadow": [],
            "tombstones": [],
            "manifest_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "build_state": "verified",
            "created_at": "2026-08-29T00:00:00+00:00",
            "verified_at": "2026-08-29T00:00:00+00:00"
        });
        let mut unicode_overlay = invalid_overlay.clone();
        unicode_overlay["files_shadow"] = serde_json::json!(["src/control-\u{0085}.rs"]);
        assert!(
            serde_json::from_value::<OverlayGenerationDescriptor>(unicode_overlay).is_ok(),
            "Rust must accept the same non-ASCII Git paths as the Python writer"
        );
        assert!(
            serde_json::from_value::<OverlayGenerationDescriptor>(invalid_overlay.clone()).is_err()
        );
        invalid_overlay["files_shadow"] = serde_json::json!([]);
        invalid_overlay["verified_at"] = serde_json::json!("not-a-timestamp");
        assert!(serde_json::from_value::<OverlayGenerationDescriptor>(invalid_overlay).is_err());
    }
}
