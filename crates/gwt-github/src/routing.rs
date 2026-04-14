//! Routing decisions for body vs. comment placement of sections.
//!
//! See SPEC-12 FR-005 and FR-006: `spec` and `tasks` default to body, every
//! other section defaults to comment, and any section whose serialized bytes
//! exceed the promote threshold is forced to comment placement. When the
//! cumulative body size still exceeds the 60 KiB headroom, the largest body
//! section is demoted repeatedly until the invariant holds.

use std::collections::BTreeMap;

use crate::body::SectionLocation;
use crate::sections::SectionName;

/// A section is auto-promoted to comment placement once its serialized size
/// exceeds this threshold.
pub const ROUTING_PROMOTE_THRESHOLD_BYTES: usize = 16 * 1024;

/// The total body budget — beyond this, sections are demoted to comments.
pub const ROUTING_BODY_BUDGET_BYTES: usize = 60 * 1024;

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
                SectionLocation::Body => sections.get(n).map(|c| c.len()),
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
                    let size = sections.get(n).map(|c| c.len()).unwrap_or(0);
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
