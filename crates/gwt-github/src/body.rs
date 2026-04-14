//! SpecBody: the composed view of an Issue body + its artifact comments.
//!
//! The [`SpecBody`] type is the in-memory representation that higher layers
//! manipulate. It bundles metadata parsed from the body header, the section
//! routing index map, and each section's raw markdown content (regardless of
//! whether the content physically lives in the body or a comment).

use std::collections::BTreeMap;
use std::fmt;

use regex::Regex;

use crate::sections::{extract_sections, SectionName, SectionParseError};

/// Metadata parsed from the body header (e.g. `<!-- gwt-spec id=2001 version=1 -->`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecMeta {
    /// Raw `id=` value from the header — an issue number or a symbolic ID
    /// used during creation.
    pub id: String,
    /// Body format version.
    pub version: u32,
}

/// Routing location for a section, captured in the body index map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionLocation {
    /// The section content is inlined in the Issue body.
    Body,
    /// The section content lives in one or more comments (by ID).
    Comments(Vec<u64>),
}

/// The body index map (`<!-- sections: ... -->`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionsIndex(pub BTreeMap<SectionName, SectionLocation>);

/// The fully-decoded [`SpecBody`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecBody {
    pub meta: SpecMeta,
    pub sections_index: SectionsIndex,
    pub sections: BTreeMap<SectionName, String>,
}

/// Errors reported by [`SpecBody::parse`].
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("missing gwt-spec header")]
    MissingHeader,
    #[error("missing sections index map")]
    MissingIndex,
    #[error("broken index map: {0}")]
    BrokenIndex(String),
    #[error(
        "comment for section '{section}' (comment:{comment_id}) not found in provided comments"
    )]
    MissingComment { section: String, comment_id: u64 },
    #[error("parts for section '{section}' are incomplete (expected {expected}, found {found})")]
    IncompleteParts {
        section: String,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Section(#[from] SectionParseError),
}

/// Input comment for [`SpecBody::parse`].
#[derive(Debug, Clone)]
pub struct Comment {
    pub id: u64,
    pub body: String,
}

impl SpecBody {
    /// Parse a body + comments snapshot into a [`SpecBody`].
    pub fn parse(body: &str, comments: &[Comment]) -> Result<Self, ParseError> {
        let meta = parse_header(body)?;
        let sections_index = parse_index_map(body)?;

        // Pre-index comments by id for O(1) lookup.
        let comment_map: BTreeMap<u64, &Comment> = comments.iter().map(|c| (c.id, c)).collect();

        // Assemble section content according to the index map.
        let mut sections: BTreeMap<SectionName, String> = BTreeMap::new();

        // First, extract any body-resident sections.
        let body_sections = extract_sections(body)?;
        let body_section_map: BTreeMap<SectionName, String> = body_sections
            .into_iter()
            .map(|s| (s.name, s.content))
            .collect();

        for (name, location) in sections_index.0.iter() {
            match location {
                SectionLocation::Body => {
                    if let Some(content) = body_section_map.get(name) {
                        sections.insert(name.clone(), content.clone());
                    } else {
                        // Body-resident but marker missing: treat as empty.
                        sections.insert(name.clone(), String::new());
                    }
                }
                SectionLocation::Comments(ids) => {
                    // Concatenate content from each referenced comment in order.
                    let mut chunks: Vec<(u32, String)> = Vec::new();
                    for &cid in ids {
                        let comment =
                            comment_map
                                .get(&cid)
                                .ok_or_else(|| ParseError::MissingComment {
                                    section: name.0.clone(),
                                    comment_id: cid,
                                })?;
                        let sections = extract_sections(&comment.body)?;
                        let matching =
                            sections
                                .into_iter()
                                .find(|s| &s.name == name)
                                .ok_or_else(|| {
                                    ParseError::BrokenIndex(format!(
                                        "comment {} does not contain section '{}'",
                                        cid, name.0
                                    ))
                                })?;
                        let index = matching.part.as_ref().map(|p| p.index).unwrap_or(1);
                        chunks.push((index, matching.content));
                    }
                    chunks.sort_by_key(|(i, _)| *i);
                    let joined = chunks
                        .into_iter()
                        .map(|(_, c)| c)
                        .collect::<Vec<_>>()
                        .join("\n");
                    sections.insert(name.clone(), joined);
                }
            }
        }

        Ok(SpecBody {
            meta,
            sections_index,
            sections,
        })
    }

    /// Replace (or insert) a section's raw content.
    ///
    /// Other sections are not touched. The routing index is not recomputed
    /// here — callers must run [`crate::routing::decide_routing`] if they
    /// need to re-evaluate body/comment placement after the edit.
    pub fn splice(&mut self, section: SectionName, content: String) {
        self.sections.insert(section, content);
    }
}

impl fmt::Display for SpecMeta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gwt-spec id={} version={}", self.id, self.version)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_header(body: &str) -> Result<SpecMeta, ParseError> {
    let re = Regex::new(r"<!--\s*gwt-spec\s+id=(?P<id>\S+)\s+version=(?P<version>\d+)\s*-->")
        .expect("valid header regex");
    let caps = re.captures(body).ok_or(ParseError::MissingHeader)?;
    let id = caps.name("id").unwrap().as_str().to_string();
    let version: u32 = caps
        .name("version")
        .unwrap()
        .as_str()
        .parse()
        .map_err(|e: std::num::ParseIntError| ParseError::BrokenIndex(e.to_string()))?;
    Ok(SpecMeta { id, version })
}

fn parse_index_map(body: &str) -> Result<SectionsIndex, ParseError> {
    // Multi-line comment block beginning with `<!-- sections:` and ending
    // with `-->` on its own. We accept content across newlines.
    let re =
        Regex::new(r"(?s)<!--\s*sections:\s*(?P<entries>.*?)\s*-->").expect("valid index regex");
    let caps = re.captures(body).ok_or(ParseError::MissingIndex)?;
    let raw_entries = caps.name("entries").unwrap().as_str();

    let mut index = SectionsIndex::default();
    for (lineno, line) in raw_entries.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once('=').ok_or_else(|| {
            ParseError::BrokenIndex(format!("line {}: '{}' missing '='", lineno + 1, line))
        })?;
        let name = SectionName(name.trim().to_string());
        let value = value.trim();
        let location = if value == "body" {
            SectionLocation::Body
        } else {
            // Expect one or more `comment:<id>` entries separated by commas.
            let mut ids: Vec<u64> = Vec::new();
            for part in value.split(',') {
                let part = part.trim();
                let id_str = part.strip_prefix("comment:").ok_or_else(|| {
                    ParseError::BrokenIndex(format!(
                        "line {}: expected 'body' or 'comment:<id>', got '{}'",
                        lineno + 1,
                        part
                    ))
                })?;
                let id: u64 = id_str.parse().map_err(|_| {
                    ParseError::BrokenIndex(format!(
                        "line {}: invalid comment id '{}'",
                        lineno + 1,
                        id_str
                    ))
                })?;
                ids.push(id);
            }
            SectionLocation::Comments(ids)
        };
        index.0.insert(name, location);
    }
    Ok(index)
}
