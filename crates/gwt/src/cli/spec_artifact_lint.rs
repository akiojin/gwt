//! Deterministic SPEC artifact lint (Issue #4541, SPEC-3248 FR-188).
//!
//! Numbering continuity, traceability cross-consistency, supersede inline
//! annotations, and section-marker / roundtrip health are all decidable by
//! machine. Leaving them to an independent reviewer agent burns inspection
//! budget on bookkeeping and detects them unreliably — the 2026-07-09 intake
//! evidence in #3248 is four missed supersede annotations found by the user,
//! not by review.
//!
//! So this module runs first, in front of [`crate::cli::intake_inspection`],
//! and turns each class into a structured [`LintFinding`]. The reviewer then
//! only sees what a machine cannot decide: scope, ownership, implementation
//! gaps, cross-SPEC contradictions.
//!
//! Everything here is pure: no I/O, no clock beyond the caller-supplied
//! `ran_at`, so the same artifact always yields the same findings in the same
//! order.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Reference prefixes whose numbering this lint tracks (FR-188 minimum set).
const TRACKED_PREFIXES: &[&str] = &["FR", "AS", "T"];

/// How much a finding blocks. `Critical` findings must carry a disposition
/// before intake completion; `Major` ones are reported but do not block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LintSeverity {
    Major,
    Critical,
}

impl LintSeverity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Critical => "critical",
        }
    }
}

/// The machine-stable class of a lint finding.
///
/// Each variant is an independent class: a traceability mismatch, a missing
/// supersede annotation, and a broken section marker never collapse into one
/// finding (Issue #4541 AC-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LintCode {
    /// The same `FR-012` is defined more than once.
    NumberingDuplicate,
    /// A run of numbers is missing between the lowest and highest definition.
    NumberingGap,
    /// A traceability table row cites a reference nothing defines.
    TraceabilityRowWithoutDefinition,
    /// A reference is defined but never appears in the traceability table.
    DefinitionWithoutTraceabilityRow,
    /// An Amendment supersedes a reference whose definition carries no inline
    /// note pointing at the Amendment.
    SupersedeAnnotationMissing,
    /// The `gwt-spec` header or `sections:` marker disagrees with the
    /// sections actually present.
    SectionMarkerBroken,
    /// A written section did not read back byte-identical.
    RoundtripMismatch,
}

impl LintCode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NumberingDuplicate => "numbering_duplicate",
            Self::NumberingGap => "numbering_gap",
            Self::TraceabilityRowWithoutDefinition => "traceability_row_without_definition",
            Self::DefinitionWithoutTraceabilityRow => "definition_without_traceability_row",
            Self::SupersedeAnnotationMissing => "supersede_annotation_missing",
            Self::SectionMarkerBroken => "section_marker_broken",
            Self::RoundtripMismatch => "roundtrip_mismatch",
        }
    }

    #[must_use]
    pub fn severity(self) -> LintSeverity {
        match self {
            Self::NumberingGap | Self::DefinitionWithoutTraceabilityRow => LintSeverity::Major,
            _ => LintSeverity::Critical,
        }
    }
}

/// One deterministic finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintFinding {
    /// Stable within one report: `L-001`, `L-002`, ...
    pub id: String,
    pub code: LintCode,
    pub severity: LintSeverity,
    /// The artifact section the finding lives in, or `"marker"` for the
    /// header, `"roundtrip"` for a write-verification mismatch.
    pub section: String,
    /// The `FR-012` / `AS-7` / `T-104` references the finding is about.
    pub refs: Vec<String>,
    pub message: String,
}

/// The result of one lint run over one owner's artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintReport {
    pub owner_number: u64,
    pub sections_scanned: Vec<String>,
    pub findings: Vec<LintFinding>,
    pub ran_at: DateTime<Utc>,
}

impl LintReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    #[must_use]
    pub fn critical_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == LintSeverity::Critical)
            .count()
    }

    /// Findings of one class, in report order.
    #[cfg(test)]
    #[must_use]
    pub fn by_code(&self, code: LintCode) -> Vec<&LintFinding> {
        self.findings
            .iter()
            .filter(|finding| finding.code == code)
            .collect()
    }
}

/// One artifact section handed to the lint.
#[derive(Debug, Clone)]
pub struct SectionInput {
    pub name: String,
    pub content: String,
}

/// Evidence that a written section read back unchanged.
#[derive(Debug, Clone)]
pub struct RoundtripObservation {
    pub section: String,
    pub written_sha256: String,
    pub readback_sha256: String,
}

/// Everything the lint reads. Assembled by the caller from the owner's cache
/// entry so the lint itself stays pure.
#[derive(Debug, Clone, Default)]
pub struct ArtifactInput {
    pub owner_number: u64,
    pub sections: Vec<SectionInput>,
    /// The Issue body prefix holding `<!-- gwt-spec ... -->` and
    /// `<!-- sections: ... -->`. `None` skips the marker check.
    pub marker_header: Option<String>,
    pub roundtrip: Vec<RoundtripObservation>,
}

/// Run every deterministic check over one artifact.
#[must_use]
pub fn lint(input: &ArtifactInput, ran_at: DateTime<Utc>) -> LintReport {
    let mut raw: Vec<(LintCode, String, Vec<String>, String)> = Vec::new();

    let definitions = collect_definitions(&input.sections);
    check_numbering(&definitions, &mut raw);
    check_traceability(&input.sections, &definitions, &mut raw);
    check_supersede_annotations(&input.sections, &definitions, &mut raw);
    check_section_markers(input, &mut raw);
    check_roundtrip(&input.roundtrip, &mut raw);

    let findings = raw
        .into_iter()
        .enumerate()
        .map(|(index, (code, section, refs, message))| LintFinding {
            id: format!("L-{:03}", index + 1),
            code,
            severity: code.severity(),
            section,
            refs,
            message,
        })
        .collect();

    LintReport {
        owner_number: input.owner_number,
        sections_scanned: input
            .sections
            .iter()
            .map(|section| section.name.clone())
            .collect(),
        findings,
        ran_at,
    }
}

/// Where one `PREFIX-NUMBER` reference is defined.
#[derive(Debug, Clone)]
struct Definition {
    section: String,
    /// 0-based line index inside the section.
    line: usize,
}

/// Every definition, keyed by prefix then number. A number defined twice keeps
/// both sites so the duplicate finding can name them.
type Definitions = BTreeMap<&'static str, BTreeMap<u32, Vec<Definition>>>;

/// A line *defines* a reference only in the three shapes this repository
/// actually uses for definitions:
///
/// - a heading — `#### FR-001 ...`,
/// - a task checkbox — `- [ ] T-001: ...`,
/// - a bold label whose bold span is *exactly* the reference —
///   `- **FR-001**: ...`, `- **AS-169**（...）: ...`.
///
/// Everything else is a mention. That distinction is what keeps the
/// traceability lists SPEC #3248 writes in its `tasks` section
/// (`- FR-191:T-313 / FR-192:T-314`) and progress notes whose bold span
/// carries prose (`- **FR-168 分類器の誤爆**: ...`) from registering as
/// second definitions of an already-defined number. It is also what lets the
/// supersede check tell a superseded target from its own inline annotation.
fn definition_subject(line: &str) -> Option<(&'static str, u32)> {
    let mut rest = line.trim_start();

    if rest.starts_with('#') {
        let text = rest.trim_start_matches('#').trim_start();
        let text = text.trim_start_matches(['*', '`']);
        let (prefix, number, _) = parse_reference(text)?;
        // `### T-635 Split the writer` defines T-635; `### FR-036 / FR-053
        // 条件` names two requirements and defines neither.
        return (references_in(text).len() == 1).then_some((prefix, number));
    }

    if let Some(stripped) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        rest = stripped.trim_start();
    } else if let Some(dot) = rest.find(". ") {
        if !rest[..dot].is_empty() && rest[..dot].bytes().all(|byte| byte.is_ascii_digit()) {
            rest = rest[dot + 2..].trim_start();
        }
    }

    for marker in ["[ ] ", "[x] ", "[X] "] {
        if let Some(stripped) = rest.strip_prefix(marker) {
            let text = stripped.trim_start().trim_start_matches(['*', '`']);
            return parse_reference(text).map(|(prefix, number, _)| (prefix, number));
        }
    }

    // Bold label. The bold span has to be the reference and nothing else.
    let bold = rest.strip_prefix("**")?;
    let end = bold.find("**")?;
    let label = bold[..end].trim();
    let (prefix, number, consumed) = parse_reference(label)?;
    (consumed == label.len()).then_some((prefix, number))
}

/// Parse a leading `PREFIX-NUMBER` token, returning the bytes it consumed.
///
/// Rejects a longer prefix (`NFR-001` is not `FR-001`) because the caller
/// always passes a token boundary, and rejects a scoped composite such as
/// `FR-4237-001`, which is a per-Issue namespace and not `FR-4237`.
fn parse_reference(text: &str) -> Option<(&'static str, u32, usize)> {
    for prefix in TRACKED_PREFIXES {
        let Some(rest) = text.strip_prefix(prefix) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('-') else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            continue;
        }
        let Ok(number) = digits.parse::<u32>() else {
            continue;
        };
        let consumed = prefix.len() + 1 + digits.len();
        if rest[digits.len()..].starts_with('-') {
            continue;
        }
        return Some((prefix, number, consumed));
    }
    None
}

/// Every `PREFIX-NUMBER` token appearing anywhere in a line, in order.
fn references_in(line: &str) -> Vec<(&'static str, u32)> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    for (index, _) in line.char_indices() {
        // Only start at a token boundary, so `NFR-001` never yields `FR-001`.
        if index > 0 {
            let prev = bytes[index - 1];
            if prev.is_ascii_alphanumeric() || prev == b'-' || prev == b'_' {
                continue;
            }
        }
        if let Some((prefix, number, _)) = parse_reference(&line[index..]) {
            found.push((prefix, number));
        }
    }
    found
}

fn format_ref(prefix: &str, number: u32) -> String {
    format!("{prefix}-{number:03}")
}

fn collect_definitions(sections: &[SectionInput]) -> Definitions {
    let mut definitions: Definitions = BTreeMap::new();
    for section in sections {
        for (line_index, line) in section.content.lines().enumerate() {
            if let Some((prefix, number)) = definition_subject(line) {
                definitions
                    .entry(prefix)
                    .or_default()
                    .entry(number)
                    .or_default()
                    .push(Definition {
                        section: section.name.clone(),
                        line: line_index,
                    });
            }
        }
    }
    definitions
}

type RawFinding = (LintCode, String, Vec<String>, String);

/// AC-1: duplicates and gaps in FR / AS / T numbering.
fn check_numbering(definitions: &Definitions, out: &mut Vec<RawFinding>) {
    for prefix in TRACKED_PREFIXES {
        let Some(numbers) = definitions.get(prefix) else {
            continue;
        };
        for (number, sites) in numbers {
            if sites.len() < 2 {
                continue;
            }
            let where_ = sites
                .iter()
                .map(|site| format!("{}:{}", site.section, site.line + 1))
                .collect::<Vec<_>>()
                .join(", ");
            out.push((
                LintCode::NumberingDuplicate,
                sites[0].section.clone(),
                vec![format_ref(prefix, *number)],
                format!(
                    "{} is defined {} times ({where_}); a duplicate number makes traceability ambiguous",
                    format_ref(prefix, *number),
                    sites.len()
                ),
            ));
        }

        // Gaps only mean something once a sequence exists.
        if numbers.len() < 2 {
            continue;
        }
        let present: BTreeSet<u32> = numbers.keys().copied().collect();
        let first = *present.iter().next().expect("non-empty");
        let last = *present.iter().next_back().expect("non-empty");
        let section = numbers
            .values()
            .next()
            .and_then(|sites| sites.first())
            .map_or_else(|| "spec".to_string(), |site| site.section.clone());
        let mut run_start: Option<u32> = None;
        for number in first..=last.saturating_add(1) {
            if number <= last && !present.contains(&number) {
                run_start.get_or_insert(number);
                continue;
            }
            let Some(start) = run_start.take() else {
                continue;
            };
            let end = number - 1;
            let refs: Vec<String> = (start..=end)
                .map(|missing| format_ref(prefix, missing))
                .collect();
            let range = if start == end {
                format_ref(prefix, start)
            } else {
                format!("{}..{}", format_ref(prefix, start), format_ref(prefix, end))
            };
            out.push((
                LintCode::NumberingGap,
                section.clone(),
                refs,
                format!(
                    "{range} is missing between {} and {}; either define it or record why the number was retired",
                    format_ref(prefix, first),
                    format_ref(prefix, last)
                ),
            ));
        }
    }
}

/// AC-2: the traceability table and the definitions must agree in both
/// directions. Runs only for prefixes the table actually carries, so a table
/// that tracks FR and T never flags every AS as untracked.
fn check_traceability(
    sections: &[SectionInput],
    definitions: &Definitions,
    out: &mut Vec<RawFinding>,
) {
    let mut table_refs: BTreeMap<&'static str, BTreeMap<u32, String>> = BTreeMap::new();
    for section in sections {
        for line in section.content.lines() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with('|') {
                continue;
            }
            for (prefix, number) in references_in(trimmed) {
                table_refs
                    .entry(prefix)
                    .or_default()
                    .entry(number)
                    .or_insert_with(|| section.name.clone());
            }
        }
    }

    // One finding per direction per prefix. A SPEC with 500 requirements and
    // a partial matrix would otherwise bury every other class under 500
    // identical rows; the full list still travels in `refs`.
    for (prefix, rows) in &table_refs {
        let defined = definitions.get(prefix);

        let orphan_rows: Vec<u32> = rows
            .keys()
            .copied()
            .filter(|number| !defined.is_some_and(|numbers| numbers.contains_key(number)))
            .collect();
        if let Some(first) = orphan_rows.first() {
            let section = rows[first].clone();
            let refs: Vec<String> = orphan_rows
                .iter()
                .map(|number| format_ref(prefix, *number))
                .collect();
            out.push((
                LintCode::TraceabilityRowWithoutDefinition,
                section,
                refs.clone(),
                format!(
                    "the traceability table cites {} reference(s) no section defines: {}",
                    refs.len(),
                    summarize_refs(&refs)
                ),
            ));
        }

        let Some(defined) = defined else {
            continue;
        };
        let untracked: Vec<u32> = defined
            .keys()
            .copied()
            .filter(|number| !rows.contains_key(number))
            .collect();
        if let Some(first) = untracked.first() {
            let section = defined[first]
                .first()
                .map_or_else(|| "spec".to_string(), |site| site.section.clone());
            let refs: Vec<String> = untracked
                .iter()
                .map(|number| format_ref(prefix, *number))
                .collect();
            out.push((
                LintCode::DefinitionWithoutTraceabilityRow,
                section,
                refs.clone(),
                format!(
                    "{} definition(s) have no traceability table row: {}",
                    refs.len(),
                    summarize_refs(&refs)
                ),
            ));
        }
    }
}

/// Keep a finding message readable when it carries hundreds of references.
fn summarize_refs(refs: &[String]) -> String {
    const SHOWN: usize = 12;
    if refs.len() <= SHOWN {
        return refs.join(", ");
    }
    format!(
        "{}, ... (+{} more)",
        refs[..SHOWN].join(", "),
        refs.len() - SHOWN
    )
}

/// AC-2: an Amendment that supersedes `FR-006` has to leave an inline note on
/// `FR-006` itself. The 2026-07-09 intake missed four of these and the user
/// found them, which is the whole reason this lint exists.
fn check_supersede_annotations(
    sections: &[SectionInput],
    definitions: &Definitions,
    out: &mut Vec<RawFinding>,
) {
    let mut targets: BTreeMap<(&'static str, u32), String> = BTreeMap::new();
    for section in sections {
        for line in section.content.lines() {
            if !mentions_supersede(line) {
                continue;
            }
            // The definition line of a reference is its own annotation.
            let subject = definition_subject(line);
            for (prefix, number) in references_in(line) {
                if subject == Some((prefix, number)) {
                    continue;
                }
                targets
                    .entry((prefix, number))
                    .or_insert_with(|| section.name.clone());
            }
        }
    }

    for ((prefix, number), citing_section) in targets {
        let Some(sites) = definitions.get(prefix).and_then(|map| map.get(&number)) else {
            // An undefined supersede target is a traceability problem, not an
            // annotation one; `check_traceability` owns that class.
            continue;
        };
        let annotated = sites.iter().any(|site| {
            sections
                .iter()
                .find(|section| section.name == site.section)
                .is_some_and(|section| definition_block(&section.content, site.line))
        });
        if annotated {
            continue;
        }
        let site = &sites[0];
        out.push((
            LintCode::SupersedeAnnotationMissing,
            site.section.clone(),
            vec![format_ref(prefix, number)],
            format!(
                "{} is superseded (cited in section '{citing_section}') but its definition at {}:{} carries no inline supersede/Amendment note",
                format_ref(prefix, number),
                site.section,
                site.line + 1
            ),
        ));
    }
}

fn mentions_supersede(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    lowered.contains("supersede")
}

/// Does the definition block starting at `line` carry a supersede annotation?
/// The block runs to the next definition, heading, or blank line.
fn definition_block(content: &str, line: usize) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    let mut index = line;
    while index < lines.len() {
        let current = lines[index];
        if index > line {
            let trimmed = current.trim();
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || definition_subject(current).is_some()
            {
                break;
            }
        }
        let lowered = current.to_ascii_lowercase();
        if lowered.contains("supersede") || lowered.contains("amendment") {
            return true;
        }
        index += 1;
    }
    false
}

/// AC-2: the `gwt-spec` header and `sections:` marker must describe exactly
/// the sections that exist, and must not be truncated mid-comment.
fn check_section_markers(input: &ArtifactInput, out: &mut Vec<RawFinding>) {
    let Some(header) = input.marker_header.as_deref() else {
        return;
    };
    let mut push = |refs: Vec<String>, message: String| {
        out.push((
            LintCode::SectionMarkerBroken,
            "marker".to_string(),
            refs,
            message,
        ));
    };

    if !header.contains("<!-- gwt-spec id=") {
        push(
            Vec::new(),
            "the artifact header has no `<!-- gwt-spec id=... -->` marker; section routing cannot be reconstructed".to_string(),
        );
        return;
    }
    let Some(marker_start) = header.find("<!-- sections:") else {
        push(
            Vec::new(),
            "the artifact header has no `<!-- sections: ... -->` marker".to_string(),
        );
        return;
    };
    let tail = &header[marker_start..];
    let Some(marker_end) = tail.find("-->") else {
        push(
            Vec::new(),
            "the `<!-- sections: ... -->` marker is unterminated; the artifact cannot round-trip"
                .to_string(),
        );
        return;
    };
    let declared: BTreeSet<String> = tail[..marker_end]
        .lines()
        .skip(1)
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            line.split_once('=')
                .map(|(name, _)| name.trim().to_string())
        })
        .collect();
    let present: BTreeSet<String> = input
        .sections
        .iter()
        .map(|section| section.name.clone())
        .collect();

    for name in declared.difference(&present) {
        push(
            Vec::new(),
            format!("the sections marker declares '{name}' but that section has no content"),
        );
    }
    for name in present.difference(&declared) {
        push(
            Vec::new(),
            format!("section '{name}' exists but the sections marker does not declare it"),
        );
    }
}

/// AC-2: a section whose readback hash differs from what was written did not
/// survive the round trip, whatever the write path reported.
fn check_roundtrip(observations: &[RoundtripObservation], out: &mut Vec<RawFinding>) {
    for observation in observations {
        if observation.written_sha256 == observation.readback_sha256 {
            continue;
        }
        out.push((
            LintCode::RoundtripMismatch,
            observation.section.clone(),
            Vec::new(),
            format!(
                "section '{}' wrote {} but read back {}; the stored artifact does not match the intended content",
                observation.section, observation.written_sha256, observation.readback_sha256
            ),
        ));
    }
}

#[cfg(test)]
mod tests;
