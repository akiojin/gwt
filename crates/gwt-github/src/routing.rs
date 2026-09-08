//! Routing decisions for body vs. comment placement of sections.
//!
//! See SPEC-12 FR-005 and FR-006: `spec` and `tasks` default to body, every
//! other section defaults to comment, and any section whose serialized bytes
//! exceed the promote threshold is forced to comment placement. When the
//! cumulative body size still exceeds the 60 KiB headroom, the largest body
//! section is demoted repeatedly until the invariant holds.

use std::collections::BTreeMap;

use crate::{body::SectionLocation, sections::SectionName};

/// A section is auto-promoted to comment placement once its serialized size
/// exceeds this threshold.
pub const ROUTING_PROMOTE_THRESHOLD_BYTES: usize = 16 * 1024;

/// The total body budget — beyond this, sections are demoted to comments.
pub const ROUTING_BODY_BUDGET_BYTES: usize = 60 * 1024;

/// Per-part content budget for comment-resident sections (SPEC-3248 P7C /
/// #3284). GitHub rejects comments beyond 65,536 characters; keeping each
/// part's *content* at 60 KiB leaves headroom for the part markers while
/// staying conservative for both byte- and character-counted limits.
pub const COMMENT_PART_BUDGET_BYTES: usize = 60 * 1024;

/// Default body-resident sections. Everything else defaults to comment.
pub const DEFAULT_BODY_SECTIONS: &[&str] = &["spec", "tasks"];

/// Output of a routing decision: a map of section name to its assigned location.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routing(pub BTreeMap<SectionName, SectionLocation>);

/// Decide routing for a set of sections given their serialized content sizes.
///
/// Rules (in order):
///
/// 1. Sections whose name is in [`DEFAULT_BODY_SECTIONS`] and whose size is
///    `<= ROUTING_PROMOTE_THRESHOLD_BYTES` default to body placement.
/// 2. All other sections default to comment placement.
/// 3. If the cumulative body size exceeds [`ROUTING_BODY_BUDGET_BYTES`], the
///    largest body-resident section is demoted repeatedly until the budget
///    is satisfied.
pub fn decide_routing(sections: &BTreeMap<SectionName, String>) -> Routing {
    let mut map: BTreeMap<SectionName, SectionLocation> = BTreeMap::new();
    for (name, content) in sections {
        let is_default_body = DEFAULT_BODY_SECTIONS.iter().any(|d| *d == name.0);
        let size = content.len();
        let location = if is_default_body && size <= ROUTING_PROMOTE_THRESHOLD_BYTES {
            SectionLocation::Body
        } else {
            SectionLocation::Comments(Vec::new())
        };
        map.insert(name.clone(), location);
    }

    // Enforce the body size budget by demoting the largest body section until
    // the total fits.
    loop {
        let body_total: usize = map
            .iter()
            .filter_map(|(n, loc)| match loc {
                SectionLocation::Body => sections.get(n).map(std::string::String::len),
                _ => None,
            })
            .sum();
        if body_total <= ROUTING_BODY_BUDGET_BYTES {
            break;
        }
        // Find the largest body section.
        let largest = map
            .iter()
            .filter_map(|(n, loc)| match loc {
                SectionLocation::Body => {
                    let size = sections.get(n).map(std::string::String::len).unwrap_or(0);
                    Some((n.clone(), size))
                }
                _ => None,
            })
            .max_by_key(|(_, size)| *size);
        match largest {
            Some((name, _)) => {
                map.insert(name, SectionLocation::Comments(Vec::new()));
            }
            None => break, // No body sections left to demote.
        }
    }

    Routing(map)
}

/// Errors reported by [`split_section_into_parts`].
#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    /// A single line is larger than the per-part budget, so no line-boundary
    /// cut can fit it. The writer must fail closed instead of truncating.
    #[error(
        "section line {line} exceeds the {budget}-byte part budget and \
         cannot be split at a line boundary"
    )]
    UnsplittableChunk { line: usize, budget: usize },
}

/// Split section content into comment-sized parts (SPEC-3248 P7C / #3284).
///
/// Cuts are made only at line boundaries. Losslessness relies on the marker
/// parser's exact-trim contract for part-marked sections: the writer wraps
/// each part as `BEGIN part=i/K` + `\n` + part + `\n` + `END part=i/K`, the
/// parser strips exactly that one newline per side, and
/// [`crate::body::SpecBody::parse`] rejoins parts with a single `\n` — the
/// same newline the cut consumed. Parts may therefore begin or end with
/// blank lines and still roundtrip byte-for-byte.
///
/// Content within [`COMMENT_PART_BUDGET_BYTES`] is returned as a single part
/// byte-for-byte. A single line larger than the budget fails closed with
/// [`SplitError::UnsplittableChunk`].
pub fn split_section_into_parts(content: &str) -> Result<Vec<String>, SplitError> {
    if content.len() <= COMMENT_PART_BUDGET_BYTES {
        return Ok(vec![content.to_string()]);
    }

    let lines: Vec<&str> = content.split('\n').collect();
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        if lines[start].len() > COMMENT_PART_BUDGET_BYTES {
            return Err(SplitError::UnsplittableChunk {
                line: start + 1,
                budget: COMMENT_PART_BUDGET_BYTES,
            });
        }
        // Grow the window greedily while the joined size fits the budget.
        let mut end = start;
        let mut size = lines[start].len();
        while end + 1 < lines.len() {
            let next = size + 1 + lines[end + 1].len();
            if next > COMMENT_PART_BUDGET_BYTES {
                break;
            }
            end += 1;
            size = next;
        }
        parts.push(lines[start..=end].join("\n"));
        start = end + 1;
    }
    Ok(parts)
}
