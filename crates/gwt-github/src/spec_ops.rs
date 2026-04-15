//! High-level SPEC operations that compose [`crate::client::IssueClient`]
//! and [`crate::cache::Cache`] into the three user-facing primitives
//! `read_section` / `write_section` / `create_spec`.
//!
//! Every public entry point routes mutating operations through the cache so
//! the invariant "API response → cache → UI" from SPEC-12 FR-022 is preserved.

use std::collections::BTreeMap;

use crate::{
    body::{Comment as BodyComment, SectionLocation, SectionsIndex, SpecMeta},
    cache::{Cache, CacheError},
    client::{
        ApiError, CommentSnapshot, FetchResult, IssueClient, IssueNumber, IssueSnapshot,
        IssueState, UpdatedAt,
    },
    routing::decide_routing,
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
    #[error("section '{0}' not found")]
    SectionNotFound(String),
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
    /// The updated content is routed via [`decide_routing`]. When the target
    /// ends up in the body, a single `patch_body` call updates the Issue.
    /// When the target is routed to a comment, the appropriate
    /// `create_comment` / `patch_comment` is emitted followed by one final
    /// `patch_body` that persists the new section index map. Cache is updated
    /// only on success; failures leave the cache untouched.
    pub fn write_section(
        &self,
        number: IssueNumber,
        name: &SectionName,
        content: &str,
    ) -> Result<(), SpecOpsError> {
        // Refresh cache to the latest snapshot before editing.
        self.refresh_cache(number)?;
        let entry = self
            .cache
            .load_entry(number)
            .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)))?;
        let mut spec_body = entry.spec_body.clone();
        spec_body.splice(name.clone(), content.to_string());

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

        // Start from the latest full body text and patch it in place.
        let mut issue_body = entry.snapshot.body.clone();
        let mut new_sections_index = spec_body.sections_index.clone();

        match (&prev_location, &new_location) {
            // Stay in body: rewrite the section between the markers, then patch.
            (Some(SectionLocation::Body), SectionLocation::Body)
            | (None, SectionLocation::Body) => {
                issue_body = rewrite_body_section(&issue_body, name, content);
                new_sections_index
                    .0
                    .insert(name.clone(), SectionLocation::Body);
                issue_body = rewrite_index_map(&issue_body, &new_sections_index);
                let _snap = self.client.patch_body(number, &issue_body)?;
            }
            // Body -> Comment promotion: create a new comment, drop the
            // body-inline markers, then patch the body with the new index map.
            (Some(SectionLocation::Body), SectionLocation::Comments(_))
            | (None, SectionLocation::Comments(_)) => {
                let comment_body = wrap_comment_body(name, content);
                let comment: CommentSnapshot = self.client.create_comment(number, &comment_body)?;
                new_sections_index
                    .0
                    .insert(name.clone(), SectionLocation::Comments(vec![comment.id.0]));
                issue_body = strip_body_section(&issue_body, name);
                issue_body = rewrite_index_map(&issue_body, &new_sections_index);
                let _snap = self.client.patch_body(number, &issue_body)?;
            }
            // Comment -> Comment: patch the first referenced comment in place.
            (Some(SectionLocation::Comments(ids)), SectionLocation::Comments(_)) => {
                if let Some(first) = ids.first().copied() {
                    let comment_body = wrap_comment_body(name, content);
                    let _patched = self
                        .client
                        .patch_comment(crate::client::CommentId(first), &comment_body)?;
                    // Routing (and existing id list) is unchanged.
                } else {
                    // Index claimed comment but no id recorded — treat as new.
                    let comment_body = wrap_comment_body(name, content);
                    let comment = self.client.create_comment(number, &comment_body)?;
                    new_sections_index
                        .0
                        .insert(name.clone(), SectionLocation::Comments(vec![comment.id.0]));
                    issue_body = rewrite_index_map(&issue_body, &new_sections_index);
                    let _snap = self.client.patch_body(number, &issue_body)?;
                }
            }
            // Comment -> Body: (rare) inline the content back into the body.
            (Some(SectionLocation::Comments(_)), SectionLocation::Body) => {
                issue_body = insert_body_section(&issue_body, name, content);
                new_sections_index
                    .0
                    .insert(name.clone(), SectionLocation::Body);
                issue_body = rewrite_index_map(&issue_body, &new_sections_index);
                let _snap = self.client.patch_body(number, &issue_body)?;
            }
        }

        // After a successful write, refresh the cache from the server so the
        // locally-assembled body and any side-effect changes remain consistent.
        self.refresh_cache(number)?;
        Ok(())
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

        // 2. For each comment-resident section, create a comment and record the id.
        let mut comment_id_map: BTreeMap<SectionName, Vec<u64>> = BTreeMap::new();
        let mut ordered: Vec<(&SectionName, &SectionLocation)> = routing.0.iter().collect();
        ordered.sort_by(|a, b| a.0.cmp(b.0));
        for (name, location) in ordered {
            if matches!(location, SectionLocation::Comments(_)) {
                let content = sections.get(name).cloned().unwrap_or_default();
                let comment_body = wrap_comment_body(name, &content);
                let snapshot = self.client.create_comment(number, &comment_body)?;
                comment_id_map.insert(name.clone(), vec![snapshot.id.0]);
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
    for (name, location) in index.0.iter() {
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
    for (name, location) in routing.0.iter() {
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

    for (name, location) in routing.0.iter() {
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
