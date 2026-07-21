//! High-level SPEC operations that compose [`crate::client::IssueClient`]
//! and [`crate::cache::Cache`] into the three user-facing primitives
//! `read_section` / `write_section` / `create_spec`.
//!
//! Every public entry point routes mutating operations through the cache so
//! the invariant "API response → cache → UI" from SPEC-12 FR-022 is preserved.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::{
    body::{Comment as BodyComment, SectionLocation, SectionsIndex, SpecMeta},
    cache::{Cache, CacheError},
    client::{
        ApiError, CommentId, CommentSnapshot, FetchResult, IssueClient, IssueNumber, IssueSnapshot,
        IssueState, UpdatedAt,
    },
    routing::{decide_routing, split_section_into_parts},
    sections::SectionName,
};

/// Errors surfaced by [`SpecOps`] operations.
#[derive(Debug, thiserror::Error)]
pub enum SpecOpsError {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Parse(#[from] crate::body::ParseError),
    #[error(transparent)]
    Split(#[from] crate::routing::SplitError),
    #[error("section '{0}' not found")]
    SectionNotFound(String),
    /// Post-write readback found remote content that differs from what was
    /// written (SPEC-3248 P7C / #3284). The write was rolled back where
    /// possible and must not be treated as committed.
    #[error(
        "post-write readback mismatch for section '{section}' — remote content \
         does not match the written content; do not trust this write"
    )]
    ReadbackMismatch { section: String },
}

/// Receipt for a committed [`SpecOps::write_section`] call (SPEC-3248 P7C /
/// #3284): canonical content size, comment part count (`0` when the section
/// is body-resident), and the SHA-256 of the canonical content that the
/// post-write readback verified against the remote copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReceipt {
    pub bytes: usize,
    pub parts: usize,
    pub sha256: String,
    /// Final resident location of the section: `"body"` or `"comments"`
    /// (SPEC-3248 P7C T-274 operability facts).
    pub location: String,
    /// Comment ids now holding the section, in part order (empty for
    /// body-resident sections).
    pub comment_ids: Vec<u64>,
    /// Largest single part payload in bytes (== `bytes` when unsplit).
    pub largest_part_bytes: usize,
}

/// High-level SPEC operations backed by an [`IssueClient`] and a [`Cache`].
pub struct SpecOps<C: IssueClient> {
    client: C,
    cache: Cache,
}

impl<C: IssueClient> SpecOps<C> {
    pub fn new(client: C, cache: Cache) -> Self {
        SpecOps { client, cache }
    }

    /// Expose the underlying client for tests and diagnostics. Higher layers
    /// should go through [`Self::read_section`] / [`Self::write_section`]
    /// instead of touching the client directly.
    pub fn client(&self) -> &C {
        &self.client
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Read a single section's content.
    ///
    /// Flow:
    /// 1. Look up the cache entry for the issue. If present, capture the
    ///    current `updated_at` as the conditional fetch key.
    /// 2. Ask the client for a fresh snapshot conditionally on that key. If
    ///    the server returns `NotModified`, the cache is already authoritative.
    /// 3. If the server returns `Updated`, atomically rewrite the cache with
    ///    the new snapshot.
    /// 4. Read the requested section file from the cache. Missing sections
    ///    surface as [`SpecOpsError::SectionNotFound`].
    pub fn read_section(
        &self,
        number: IssueNumber,
        name: &SectionName,
    ) -> Result<String, SpecOpsError> {
        self.refresh_cache(number)?;
        match self.cache.read_section(number, name)? {
            Some(s) => Ok(s),
            None => Err(SpecOpsError::SectionNotFound(name.0.clone())),
        }
    }

    /// Replace the content of a section.
    ///
    /// The updated content is canonicalized (surrounding newlines trimmed,
    /// matching what a later parse returns) and routed via
    /// [`decide_routing`]. Comment-resident content is split into
    /// comment-sized parts by [`split_section_into_parts`] (SPEC-3248 P7C /
    /// #3284) and written with `part=N/M` markers.
    ///
    /// Write protocol for part-count-changing comment writes is
    /// create-then-swap-then-delete: new part comments are created first, a
    /// single `patch_body` atomically swaps the section index to the new
    /// comment ids, and stale comments are deleted only after the post-write
    /// readback verifies the remote content. A failure before the index swap
    /// leaves readers on the previous content (zero partial overwrite); a
    /// readback mismatch rolls the body back and fails closed.
    pub fn write_section(
        &self,
        number: IssueNumber,
        name: &SectionName,
        content: &str,
    ) -> Result<WriteReceipt, SpecOpsError> {
        // Refresh cache to the latest snapshot before editing.
        self.refresh_cache(number)?;
        let entry = self
            .cache
            .load_entry(number)
            .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)))?;
        let canonical = crate::sections::trim_surrounding_newlines(content).to_string();
        let mut spec_body = entry.spec_body.clone();
        spec_body.splice(name.clone(), canonical.clone());

        // Recompute routing from the new section map.
        let new_routing = decide_routing(&spec_body.sections);

        // Read the previous location (if any) so we can decide between
        // create_comment / patch_comment / patch_body.
        let prev_location = spec_body.sections_index.0.get(name).cloned();

        let new_location = new_routing
            .0
            .get(name)
            .cloned()
            .unwrap_or(SectionLocation::Body);

        // Start from the latest full body text and patch it in place. Keep
        // the original for rollback after a readback mismatch.
        let original_body = entry.snapshot.body.clone();
        let mut issue_body = entry.snapshot.body;
        let mut new_sections_index = spec_body.sections_index.clone();

        let prev_ids: Vec<u64> = match &prev_location {
            Some(SectionLocation::Comments(ids)) => ids.clone(),
            _ => Vec::new(),
        };

        // Comments created by this write (rolled back on readback mismatch)
        // and stale comments to delete after a verified swap.
        let mut created_ids: Vec<u64> = Vec::new();
        let mut stale_ids: Vec<u64> = Vec::new();
        // Whether rolling back means restoring the original body text.
        let mut rollback_body = false;
        let parts_written: usize;
        let mut largest_part_bytes = canonical.len();

        match (&prev_location, &new_location) {
            // Stay in body: rewrite the section between the markers, then patch.
            (Some(SectionLocation::Body) | None, SectionLocation::Body) => {
                issue_body = rewrite_body_section(&issue_body, name, &canonical);
                new_sections_index
                    .0
                    .insert(name.clone(), SectionLocation::Body);
                issue_body = rewrite_index_map(&issue_body, &new_sections_index);
                let _snap = self.client.patch_body(number, &issue_body)?;
                rollback_body = true;
                parts_written = 0;
            }
            // Comment -> Comment, single part staying single: patch the
            // existing comment in place (stable comment id, single atomic
            // mutation).
            (Some(SectionLocation::Comments(ids)), SectionLocation::Comments(_))
                if ids.len() == 1
                    && canonical.len() <= crate::routing::COMMENT_PART_BUDGET_BYTES =>
            {
                let comment_body = wrap_comment_part_body(name, &canonical, 1, 1);
                let _patched = self
                    .client
                    .patch_comment(CommentId(ids[0]), &comment_body)?;
                parts_written = 1;
            }
            // Every other comment-resident shape (promotion from body, part
            // count changes, or an index entry with no recorded id):
            // create-then-swap-then-delete.
            (_, SectionLocation::Comments(_)) => {
                let parts = split_section_into_parts(&canonical)?;
                let total = parts.len();
                largest_part_bytes = parts.iter().map(String::len).max().unwrap_or(0);
                for (i, part) in parts.iter().enumerate() {
                    let comment_body = wrap_comment_part_body(name, part, i + 1, total);
                    let comment: CommentSnapshot =
                        self.client.create_comment(number, &comment_body)?;
                    created_ids.push(comment.id.0);
                }
                new_sections_index
                    .0
                    .insert(name.clone(), SectionLocation::Comments(created_ids.clone()));
                if matches!(&prev_location, Some(SectionLocation::Body)) {
                    issue_body = strip_body_section(&issue_body, name);
                }
                issue_body = rewrite_index_map(&issue_body, &new_sections_index);
                let _snap = self.client.patch_body(number, &issue_body)?;
                rollback_body = true;
                stale_ids = prev_ids;
                parts_written = total;
            }
            // Comment -> Body: (rare) inline the content back into the body.
            (Some(SectionLocation::Comments(_)), SectionLocation::Body) => {
                issue_body = insert_body_section(&issue_body, name, &canonical);
                new_sections_index
                    .0
                    .insert(name.clone(), SectionLocation::Body);
                issue_body = rewrite_index_map(&issue_body, &new_sections_index);
                let _snap = self.client.patch_body(number, &issue_body)?;
                rollback_body = true;
                stale_ids = prev_ids;
                parts_written = 0;
            }
        }

        // Post-write readback: refetch the issue unconditionally and verify
        // the section now parses back to exactly the canonical content.
        let readback_ok = self.readback_section(number, name, &canonical)?;
        if !readback_ok {
            // Roll back: restore the original body (and with it the original
            // section index), then drop any comments this write created.
            if rollback_body {
                let _ = self.client.patch_body(number, &original_body);
            }
            for id in created_ids {
                let _ = self.client.delete_comment(CommentId(id));
            }
            // Leave the cache on the (restored) remote state.
            let _ = self.force_refresh_cache(number);
            return Err(SpecOpsError::ReadbackMismatch {
                section: name.0.clone(),
            });
        }

        // Verified: clean up stale comments that are no longer referenced by
        // the swapped index. Deletion failures leave harmless orphans (the
        // index no longer references them), so they are best-effort.
        for id in stale_ids {
            let _ = self.client.delete_comment(CommentId(id));
        }
        self.force_refresh_cache(number)?;

        let (location, comment_ids) = match new_sections_index.0.get(name) {
            Some(SectionLocation::Comments(ids)) => ("comments".to_string(), ids.clone()),
            _ => ("body".to_string(), Vec::new()),
        };
        Ok(WriteReceipt {
            bytes: canonical.len(),
            parts: parts_written,
            sha256: format!("{:x}", Sha256::digest(canonical.as_bytes())),
            location,
            comment_ids,
            largest_part_bytes,
        })
    }

    /// Create a brand-new SPEC.
    ///
    /// The caller provides the section map (keyed by section name) and any
    /// additional labels. The `gwt-spec` label is always included. The call
    /// sequence is `create_issue` → `create_comment` × N (for sections routed
    /// to comments) → final `patch_body` to write the completed index map.
    pub fn create_spec(
        &self,
        title: &str,
        sections: BTreeMap<SectionName, String>,
        extra_labels: &[String],
    ) -> Result<IssueSnapshot, SpecOpsError> {
        let routing = decide_routing(&sections);

        // Compose the initial body with body-inline sections, but leave
        // comment-resident sections as `pending` placeholders in the index.
        let initial_body = render_body(
            &SpecMeta {
                id: "new".to_string(),
                version: 1,
            },
            &routing,
            &sections,
            &BTreeMap::new(),
        );

        let mut labels: Vec<String> = vec!["gwt-spec".to_string()];
        for l in extra_labels {
            if !labels.iter().any(|existing| existing == l) {
                labels.push(l.clone());
            }
        }

        // 1. Create the Issue with the initial body.
        let created = self.client.create_issue(title, &initial_body, &labels)?;
        let number = created.number;

        // 2. For each comment-resident section, create one comment per part
        //    (SPEC-3248 P7C / #3284) and record the ids in part order.
        let mut comment_id_map: BTreeMap<SectionName, Vec<u64>> = BTreeMap::new();
        let mut ordered: Vec<(&SectionName, &SectionLocation)> = routing.0.iter().collect();
        ordered.sort_by(|a, b| a.0.cmp(b.0));
        for (name, location) in ordered {
            if matches!(location, SectionLocation::Comments(_)) {
                let content = sections.get(name).cloned().unwrap_or_default();
                let canonical = crate::sections::trim_surrounding_newlines(&content).to_string();
                let parts = split_section_into_parts(&canonical)?;
                let total = parts.len();
                let mut ids: Vec<u64> = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    let comment_body = wrap_comment_part_body(name, part, i + 1, total);
                    let snapshot = self.client.create_comment(number, &comment_body)?;
                    ids.push(snapshot.id.0);
                }
                comment_id_map.insert(name.clone(), ids);
            }
        }

        // 3. Final patch: rewrite the body with the resolved comment ids.
        let final_body = render_body(
            &SpecMeta {
                id: number.0.to_string(),
                version: 1,
            },
            &routing,
            &sections,
            &comment_id_map,
        );
        let _patched = self.client.patch_body(number, &final_body)?;

        // 4. Refresh the cache snapshot.
        let fresh = match self.client.fetch(number, None)? {
            FetchResult::Updated(s) => s,
            FetchResult::NotModified => created,
        };
        self.cache.write_snapshot(&fresh)?;

        Ok(fresh)
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    fn refresh_cache(&self, number: IssueNumber) -> Result<(), SpecOpsError> {
        let entry = self.cache.load_entry(number);
        let since: Option<UpdatedAt> = entry.as_ref().map(|e| e.snapshot.updated_at.clone());
        let res = self.client.fetch(number, since.as_ref())?;
        match res {
            FetchResult::NotModified => Ok(()),
            FetchResult::Updated(snapshot) => {
                self.cache.write_snapshot(&snapshot)?;
                Ok(())
            }
        }
    }

    /// Unconditional refresh: refetch the issue without a conditional key so
    /// same-timestamp mutations (GitHub `updatedAt` has second granularity)
    /// cannot leave the cache stale during post-write readback.
    fn force_refresh_cache(&self, number: IssueNumber) -> Result<(), SpecOpsError> {
        match self.client.fetch(number, None)? {
            FetchResult::Updated(snapshot) => {
                self.cache.write_snapshot(&snapshot)?;
                Ok(())
            }
            FetchResult::NotModified => Ok(()),
        }
    }

    /// Post-write readback (SPEC-3248 P7C / #3284): refetch the remote issue
    /// and confirm the section parses back to exactly `expected`.
    fn readback_section(
        &self,
        number: IssueNumber,
        name: &SectionName,
        expected: &str,
    ) -> Result<bool, SpecOpsError> {
        self.force_refresh_cache(number)?;
        Ok(self.cache.read_section(number, name)?.as_deref() == Some(expected))
    }
}

// ---------------------------------------------------------------------------
// Body text manipulation helpers
// ---------------------------------------------------------------------------

fn rewrite_body_section(body: &str, name: &SectionName, new_content: &str) -> String {
    let begin = format!("<!-- artifact:{} BEGIN -->", name.0);
    let end = format!("<!-- artifact:{} END -->", name.0);
    let Some(begin_idx) = body.find(&begin) else {
        return insert_body_section(body, name, new_content);
    };
    let search_from = begin_idx + begin.len();
    let Some(end_rel) = body[search_from..].find(&end) else {
        return body.to_string();
    };
    let end_idx = search_from + end_rel;
    let mut out = String::with_capacity(body.len() + new_content.len());
    out.push_str(&body[..begin_idx]);
    out.push_str(&begin);
    out.push('\n');
    out.push_str(new_content.trim_end_matches('\n'));
    out.push('\n');
    out.push_str(&end);
    out.push_str(&body[end_idx + end.len()..]);
    out
}

fn insert_body_section(body: &str, name: &SectionName, content: &str) -> String {
    // Append a fresh section block before the trailing newline.
    let mut out = body.trim_end_matches('\n').to_string();
    out.push_str("\n\n");
    out.push_str(&format!("<!-- artifact:{} BEGIN -->\n", name.0));
    out.push_str(content.trim_end_matches('\n'));
    out.push('\n');
    out.push_str(&format!("<!-- artifact:{} END -->\n", name.0));
    out
}

fn strip_body_section(body: &str, name: &SectionName) -> String {
    let begin = format!("<!-- artifact:{} BEGIN -->", name.0);
    let end = format!("<!-- artifact:{} END -->", name.0);
    let Some(begin_idx) = body.find(&begin) else {
        return body.to_string();
    };
    let search_from = begin_idx + begin.len();
    let Some(end_rel) = body[search_from..].find(&end) else {
        return body.to_string();
    };
    let end_idx = search_from + end_rel + end.len();
    let mut out = String::with_capacity(body.len());
    out.push_str(body[..begin_idx].trim_end_matches(['\n', ' ']));
    out.push('\n');
    out.push_str(body[end_idx..].trim_start_matches('\n'));
    out
}

fn rewrite_index_map(body: &str, index: &SectionsIndex) -> String {
    // Render a new index block and replace the existing one.
    let mut rendered = String::from("<!-- sections:\n");
    for (name, location) in &index.0 {
        match location {
            SectionLocation::Body => {
                rendered.push_str(&format!("{}=body\n", name.0));
            }
            SectionLocation::Comments(ids) => {
                let joined = ids
                    .iter()
                    .map(|i| format!("comment:{i}"))
                    .collect::<Vec<_>>()
                    .join(",");
                rendered.push_str(&format!("{}={joined}\n", name.0));
            }
        }
    }
    rendered.push_str("-->");

    let re = regex::Regex::new(r"(?s)<!--\s*sections:.*?-->").expect("valid regex");
    re.replace(body, rendered.as_str()).to_string()
}

fn wrap_comment_body(name: &SectionName, content: &str) -> String {
    let trimmed = content.trim_end_matches('\n');
    format!(
        "<!-- artifact:{name} BEGIN -->\n{trimmed}\n<!-- artifact:{name} END -->",
        name = name.0
    )
}

/// Wrap one part of a (possibly multipart) comment-resident section. A
/// single-part section keeps the unmarked legacy format so older readers stay
/// compatible; multipart sections carry `part=N/M` markers under the parser's
/// exact-trim contract, so the part content must not be re-trimmed here — a
/// part may legitimately begin or end with blank lines that belong to the
/// section content.
fn wrap_comment_part_body(name: &SectionName, content: &str, index: usize, total: usize) -> String {
    if total <= 1 {
        return wrap_comment_body(name, content);
    }
    format!(
        "<!-- artifact:{name} BEGIN part={index}/{total} -->\n{content}\n<!-- artifact:{name} END part={index}/{total} -->",
        name = name.0
    )
}

fn render_body(
    meta: &SpecMeta,
    routing: &crate::routing::Routing,
    sections: &BTreeMap<SectionName, String>,
    comment_ids: &BTreeMap<SectionName, Vec<u64>>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "<!-- gwt-spec id={} version={} -->\n",
        meta.id, meta.version
    ));
    out.push_str("<!-- sections:\n");
    for (name, location) in &routing.0 {
        match location {
            SectionLocation::Body => {
                out.push_str(&format!("{}=body\n", name.0));
            }
            SectionLocation::Comments(_) => {
                let ids = comment_ids.get(name).cloned().unwrap_or_default();
                if ids.is_empty() {
                    out.push_str(&format!("{}=comment:pending\n", name.0));
                } else {
                    let joined = ids
                        .iter()
                        .map(|i| format!("comment:{i}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    out.push_str(&format!("{}={joined}\n", name.0));
                }
            }
        }
    }
    out.push_str("-->\n\n");

    for (name, location) in &routing.0 {
        if let SectionLocation::Body = location {
            if let Some(content) = sections.get(name) {
                out.push_str(&format!("<!-- artifact:{} BEGIN -->\n", name.0));
                out.push_str(content.trim_end_matches('\n'));
                out.push('\n');
                out.push_str(&format!("<!-- artifact:{} END -->\n\n", name.0));
            }
        }
    }

    out
}

// Keep these types used so clippy does not complain about unused imports in
// configurations where only some of them are touched.
#[allow(dead_code)]
fn _keep_types_used(_c: BodyComment, _s: IssueState) {}
