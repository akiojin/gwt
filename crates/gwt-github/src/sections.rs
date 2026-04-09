//! Section marker parser for SPEC-12 hybrid body/comment storage.
//!
//! This module is responsible for locating `<!-- artifact:NAME BEGIN -->` /
//! `<!-- artifact:NAME END -->` marker pairs inside an Issue body or comment
//! body, and returning the extracted section name and content in source order.
//! It also recognises the `part=N/M` suffix used when a single section has
//! been split across multiple comments to stay under the GitHub 65,536 byte
//! comment limit.

use std::fmt;

use regex::Regex;

/// Semantic-agnostic section identifier.
///
/// The parser does not interpret the section name; higher layers are free to
/// map strings such as `"spec"`, `"plan"`, `"contract/api.yaml"`, or
/// `"checklist/tdd.md"` onto domain-specific types.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct SectionName(pub String);

impl fmt::Display for SectionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Part marker for a section split across multiple comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionPart {
    /// 1-based part index.
    pub index: u32,
    /// Total number of parts for the section.
    pub total: u32,
}

/// A section extracted from a raw Issue body or comment body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSection {
    pub name: SectionName,
    pub content: String,
    pub part: Option<SectionPart>,
}

/// Errors reported by the section parser.
#[derive(Debug, thiserror::Error)]
pub enum SectionParseError {
    /// A `BEGIN` marker was found with no matching `END`.
    #[error("section '{0}' has BEGIN without matching END")]
    UnterminatedSection(String),

    /// An `END` marker was found with no preceding `BEGIN`.
    #[error("section '{0}' has END without matching BEGIN")]
    UnmatchedEnd(String),

    /// The same section name appeared more than once at the same part
    /// position within a single input.
    #[error("section '{0}' appears more than once")]
    DuplicateSection(String),

    /// A marker had an unrecognised or inconsistent shape, such as a BEGIN
    /// that declares a part but the matching END does not.
    #[error("malformed marker: {0}")]
    MalformedMarker(String),
}

/// Parse an Issue body or comment body and return every section it contains
/// in source order.
pub fn extract_sections(text: &str) -> Result<Vec<ExtractedSection>, SectionParseError> {
    let begin_re = Regex::new(
        r"(?m)<!--\s*artifact:(?P<name>[A-Za-z0-9_./\-]+)\s+BEGIN(?:\s+part=(?P<idx>\d+)/(?P<total>\d+))?\s*-->",
    )
    .expect("valid begin regex");
    let end_re = Regex::new(
        r"(?m)<!--\s*artifact:(?P<name>[A-Za-z0-9_./\-]+)\s+END(?:\s+part=(?P<idx>\d+)/(?P<total>\d+))?\s*-->",
    )
    .expect("valid end regex");

    // Collect all markers with their positions in a single pass so we can walk
    // them in source order.
    #[derive(Debug)]
    enum Kind {
        Begin,
        End,
    }
    #[derive(Debug)]
    struct Marker<'a> {
        kind: Kind,
        name: &'a str,
        part: Option<SectionPart>,
        marker_start: usize,
        marker_end: usize,
    }

    let mut markers: Vec<Marker<'_>> = Vec::new();
    for cap in begin_re.captures_iter(text) {
        let full = cap.get(0).expect("begin full match");
        let name = cap.name("name").expect("begin name").as_str();
        let part =
            match (cap.name("idx"), cap.name("total")) {
                (Some(i), Some(t)) => {
                    let index: u32 = i.as_str().parse().map_err(|_| {
                        SectionParseError::MalformedMarker(full.as_str().to_string())
                    })?;
                    let total: u32 = t.as_str().parse().map_err(|_| {
                        SectionParseError::MalformedMarker(full.as_str().to_string())
                    })?;
                    Some(SectionPart { index, total })
                }
                (None, None) => None,
                _ => {
                    return Err(SectionParseError::MalformedMarker(
                        full.as_str().to_string(),
                    ))
                }
            };
        markers.push(Marker {
            kind: Kind::Begin,
            name,
            part,
            marker_start: full.start(),
            marker_end: full.end(),
        });
    }
    for cap in end_re.captures_iter(text) {
        let full = cap.get(0).expect("end full match");
        let name = cap.name("name").expect("end name").as_str();
        let part =
            match (cap.name("idx"), cap.name("total")) {
                (Some(i), Some(t)) => {
                    let index: u32 = i.as_str().parse().map_err(|_| {
                        SectionParseError::MalformedMarker(full.as_str().to_string())
                    })?;
                    let total: u32 = t.as_str().parse().map_err(|_| {
                        SectionParseError::MalformedMarker(full.as_str().to_string())
                    })?;
                    Some(SectionPart { index, total })
                }
                (None, None) => None,
                _ => {
                    return Err(SectionParseError::MalformedMarker(
                        full.as_str().to_string(),
                    ))
                }
            };
        markers.push(Marker {
            kind: Kind::End,
            name,
            part,
            marker_start: full.start(),
            marker_end: full.end(),
        });
    }
    markers.sort_by_key(|m| m.marker_start);

    let mut out: Vec<ExtractedSection> = Vec::new();
    let mut iter = markers.into_iter().peekable();
    while let Some(marker) = iter.next() {
        match marker.kind {
            Kind::End => {
                return Err(SectionParseError::UnmatchedEnd(marker.name.to_string()));
            }
            Kind::Begin => {
                let begin_name = marker.name.to_string();
                let begin_part = marker.part.clone();
                let body_start = marker.marker_end;
                // The next marker in source order must be the matching END.
                let next = iter
                    .next()
                    .ok_or_else(|| SectionParseError::UnterminatedSection(begin_name.clone()))?;
                let end_marker = match next.kind {
                    Kind::End if next.name == begin_name => {
                        if next.part != begin_part {
                            return Err(SectionParseError::MalformedMarker(format!(
                                "BEGIN/END part mismatch for '{}'",
                                begin_name
                            )));
                        }
                        next
                    }
                    Kind::End => {
                        return Err(SectionParseError::UnmatchedEnd(next.name.to_string()));
                    }
                    Kind::Begin => {
                        return Err(SectionParseError::MalformedMarker(format!(
                            "nested BEGIN for '{}' inside '{}'",
                            next.name, begin_name
                        )));
                    }
                };
                let body_end = end_marker.marker_start;
                let raw = &text[body_start..body_end];
                let content = trim_surrounding_newlines(raw).to_string();
                // Duplicate detection: same name AND same part.
                let is_duplicate = out
                    .iter()
                    .any(|s| s.name.0 == begin_name && s.part == begin_part);
                if is_duplicate {
                    return Err(SectionParseError::DuplicateSection(begin_name));
                }
                out.push(ExtractedSection {
                    name: SectionName(begin_name),
                    content,
                    part: begin_part,
                });
            }
        }
    }
    Ok(out)
}

/// Strip at most the leading/trailing newline characters from the section
/// body without touching interior whitespace.
fn trim_surrounding_newlines(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    let mut start = 0usize;
    let mut end = bytes.len();
    // Strip leading \n and \r\n sequences.
    while start < end {
        match bytes[start] {
            b'\n' => start += 1,
            b'\r' if start + 1 < end && bytes[start + 1] == b'\n' => start += 2,
            _ => break,
        }
    }
    // Strip trailing \n and \r\n sequences.
    while end > start {
        match bytes[end - 1] {
            b'\n' => {
                end -= 1;
                if end > start && bytes[end - 1] == b'\r' {
                    end -= 1;
                }
            }
            _ => break,
        }
    }
    &raw[start..end]
}
