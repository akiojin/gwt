//! Structured spec JSON model + render / merge / split rules (SPEC-3060).
//!
//! Moved here from the gwtd CLI (`cli::issue_spec::structured`, originally
//! split out for SPEC-1942 SC-027) so that any client — CLI, GUI, future
//! tools — shares one owner for the structured editing schema: the
//! [`StructuredSpecInput`] types parsed from `--edit spec --json` payloads,
//! the renderer that reconstructs canonical Markdown sections, and the
//! merge / split helpers that allow incremental updates of an existing
//! spec body. Input IO (stdin / file reading) stays with each client.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{client::ApiError, SpecOpsError};

/// The canonical spec body sections, in render order. `merge` replaces
/// these in place and preserves any other (`unknown`) section verbatim.
pub const CANONICAL_SECTION_HEADINGS: [&str; 6] = [
    "Background",
    "User Stories",
    "Edge Cases",
    "Functional Requirements",
    "Non-Functional Requirements",
    "Success Criteria",
];

/// Free-text block accepted either as one string or as a paragraph list.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TextBlock {
    /// A single (possibly multi-line) string.
    Text(String),
    /// Multiple paragraphs joined with a blank line on render.
    Paragraphs(Vec<String>),
}

/// Partial structured spec payload. Every field is optional: `Some` means
/// "replace this canonical section with the rendered content (or remove it
/// when the rendered content is empty)", `None` means "leave it untouched".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StructuredSpecInput {
    /// `## Background` content.
    #[serde(default)]
    pub background: Option<TextBlock>,
    /// `## User Stories` entries (renumbered `US-1`, `US-2`, … on render).
    #[serde(default)]
    pub user_stories: Option<Vec<StructuredUserStory>>,
    /// `## Edge Cases` bullet list.
    #[serde(default)]
    pub edge_cases: Option<Vec<String>>,
    /// `## Functional Requirements` (renumbered `FR-001`, …).
    #[serde(default)]
    pub functional_requirements: Option<Vec<String>>,
    /// `## Non-Functional Requirements` (renumbered `NFR-001`, …).
    #[serde(default)]
    pub non_functional_requirements: Option<Vec<String>>,
    /// `## Success Criteria` (renumbered `SC-001`, …).
    #[serde(default)]
    pub success_criteria: Option<Vec<String>>,
}

/// One user story in a [`StructuredSpecInput`]. Either provide a full
/// `statement`, or the `as_a` / `i_want` / `so_that` triple from which the
/// statement is composed.
#[derive(Debug, Clone, Deserialize)]
pub struct StructuredUserStory {
    /// Story title; a leading `US-n:` label is stripped and renumbered.
    pub title: String,
    /// Priority; normalized to a `P` prefix (`"1"` → `P1`).
    #[serde(default)]
    pub priority: Option<String>,
    /// Optional status suffix rendered after the priority.
    #[serde(default)]
    pub status: Option<String>,
    /// Full story statement; wins over the `as_a` triple when present.
    #[serde(default)]
    pub statement: Option<String>,
    /// "As &lt;role&gt;" fragment.
    #[serde(default)]
    pub as_a: Option<String>,
    /// "I want &lt;capability&gt;" fragment.
    #[serde(default)]
    pub i_want: Option<String>,
    /// "so that &lt;benefit&gt;" fragment.
    #[serde(default)]
    pub so_that: Option<String>,
    /// Acceptance scenarios; list markers are stripped and renumbered.
    #[serde(default, alias = "acceptance")]
    pub acceptance_scenarios: Vec<String>,
}

/// Parse a structured spec JSON payload.
pub fn parse_structured_spec_json(raw: &str) -> Result<StructuredSpecInput, SpecOpsError> {
    serde_json::from_str(raw).map_err(|err| {
        SpecOpsError::from(ApiError::Unexpected(format!("invalid spec json: {err}")))
    })
}

/// Render a complete canonical spec body (`# title` + present sections).
pub fn render_structured_spec(title: &str, structured: &StructuredSpecInput) -> String {
    let mut parts = vec![format!("# {}", title.trim())];
    if let Some(section) = structured
        .background
        .as_ref()
        .and_then(render_background_section)
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .user_stories
        .as_ref()
        .and_then(|stories| render_user_stories_section(stories))
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .edge_cases
        .as_ref()
        .and_then(|items| render_bullet_section("Edge Cases", items))
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .functional_requirements
        .as_ref()
        .and_then(|items| {
            render_numbered_requirement_section("Functional Requirements", "FR", items)
        })
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .non_functional_requirements
        .as_ref()
        .and_then(|items| {
            render_numbered_requirement_section("Non-Functional Requirements", "NFR", items)
        })
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .success_criteria
        .as_ref()
        .and_then(|items| render_numbered_requirement_section("Success Criteria", "SC", items))
    {
        parts.push(section);
    }
    parts.join("\n\n").trim_end().to_string() + "\n"
}

/// Apply a structured patch to an existing spec body: canonical sections in
/// the patch are replaced (or removed when they render empty), all other
/// sections are preserved verbatim after the canonical block.
pub fn merge_structured_spec(existing: &str, patch: &StructuredSpecInput) -> String {
    let (title, known_sections, unknown_sections) = split_structured_spec(existing);
    let mut merged = known_sections;
    for (heading, value) in rendered_sections_for_patch(patch) {
        match value {
            Some(content) => {
                merged.insert(heading.to_string(), content);
            }
            None => {
                merged.remove(heading);
            }
        }
    }

    let mut parts = vec![format!("# {}", title.trim())];
    for heading in CANONICAL_SECTION_HEADINGS {
        if let Some(section) = merged.get(heading) {
            parts.push(section.clone());
        }
    }
    for section in unknown_sections {
        parts.push(section);
    }
    parts.join("\n\n").trim_end().to_string() + "\n"
}

/// Render every canonical section present in `patch`, pairing each heading
/// with `None` when the section content renders empty (= remove it).
pub fn rendered_sections_for_patch(
    patch: &StructuredSpecInput,
) -> Vec<(&'static str, Option<String>)> {
    let mut sections = Vec::new();
    if let Some(background) = patch.background.as_ref() {
        sections.push(("Background", render_background_section(background)));
    }
    if let Some(user_stories) = patch.user_stories.as_ref() {
        sections.push(("User Stories", render_user_stories_section(user_stories)));
    }
    if let Some(edge_cases) = patch.edge_cases.as_ref() {
        sections.push((
            "Edge Cases",
            render_bullet_section("Edge Cases", edge_cases),
        ));
    }
    if let Some(requirements) = patch.functional_requirements.as_ref() {
        sections.push((
            "Functional Requirements",
            render_numbered_requirement_section("Functional Requirements", "FR", requirements),
        ));
    }
    if let Some(requirements) = patch.non_functional_requirements.as_ref() {
        sections.push((
            "Non-Functional Requirements",
            render_numbered_requirement_section("Non-Functional Requirements", "NFR", requirements),
        ));
    }
    if let Some(criteria) = patch.success_criteria.as_ref() {
        sections.push((
            "Success Criteria",
            render_numbered_requirement_section("Success Criteria", "SC", criteria),
        ));
    }
    sections
}

/// Split an existing spec body into its title, the canonical sections (by
/// heading), and every unknown section in document order.
pub fn split_structured_spec(existing: &str) -> (String, BTreeMap<String, String>, Vec<String>) {
    let title = extract_document_title(existing).unwrap_or_else(|| "Specification".to_string());
    let mut known_sections = BTreeMap::new();
    let mut unknown_sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    let mut flush = |heading: Option<String>, lines: &mut Vec<String>| {
        if let Some(name) = heading {
            let section = lines.join("\n").trim().to_string();
            if !section.is_empty() {
                if CANONICAL_SECTION_HEADINGS.contains(&name.as_str()) {
                    known_sections.insert(name, section);
                } else {
                    unknown_sections.push(section);
                }
            }
            lines.clear();
        }
    };

    for line in existing.lines() {
        if line.starts_with("# ") && current_heading.is_none() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            let previous_heading = current_heading.take();
            flush(previous_heading, &mut current_lines);
            current_heading = Some(rest.trim().to_string());
            current_lines.push(line.to_string());
        } else if current_heading.is_some() {
            current_lines.push(line.to_string());
        }
    }
    flush(current_heading.take(), &mut current_lines);

    (title, known_sections, unknown_sections)
}

/// First `# ` heading of `markdown`, when present and non-empty.
pub fn extract_document_title(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

/// Strip a leading `SPEC:` / `SPEC-n:` label from an issue title so it can
/// be used as the document heading.
pub fn normalize_spec_heading_from_title(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(rest) = trimmed.strip_prefix("SPEC:") {
        return rest.trim().to_string();
    }
    if let Some(colon) = trimmed.find(':') {
        let head = trimmed[..colon].trim();
        if head.eq_ignore_ascii_case("SPEC") || head.starts_with("SPEC-") {
            return trimmed[colon + 1..].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Render the `## Background` section, or `None` when the block is empty.
pub fn render_background_section(background: &TextBlock) -> Option<String> {
    let content = normalize_text_block(background)?;
    Some(format!("## Background\n\n{content}"))
}

/// Render the `## User Stories` section with renumbered `US-n` headings.
pub fn render_user_stories_section(user_stories: &[StructuredUserStory]) -> Option<String> {
    let rendered: Vec<String> = user_stories
        .iter()
        .enumerate()
        .filter_map(|(index, story)| render_user_story(index + 1, story))
        .collect();
    if rendered.is_empty() {
        None
    } else {
        Some(format!("## User Stories\n\n{}", rendered.join("\n\n")))
    }
}

/// Render one user story (`### US-{index}: …`), or `None` when untitled.
pub fn render_user_story(index: usize, story: &StructuredUserStory) -> Option<String> {
    let title = normalize_user_story_title(&story.title);
    if title.is_empty() {
        return None;
    }
    let mut header = format!("### US-{index}: {title}");
    if let Some(priority) = story
        .priority
        .as_deref()
        .map(normalize_priority)
        .filter(|value| !value.is_empty())
    {
        header.push_str(&format!(" ({priority})"));
    }
    if let Some(status) = story
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        header.push_str(&format!(" -- {status}"));
    }

    let statement = story
        .statement
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| build_user_story_statement(story))
        .unwrap_or_else(|| "[NEEDS CLARIFICATION: add a user story statement]".to_string());

    let scenarios: Vec<String> = story
        .acceptance_scenarios
        .iter()
        .map(|scenario| strip_list_marker(scenario))
        .filter(|scenario| !scenario.is_empty())
        .collect();

    let mut parts = vec![header, String::new(), statement];
    if !scenarios.is_empty() {
        parts.push(String::new());
        parts.push("**Acceptance Scenarios:**".to_string());
        parts.push(String::new());
        parts.extend(
            scenarios
                .into_iter()
                .enumerate()
                .map(|(idx, scenario)| format!("{}. {scenario}", idx + 1)),
        );
    }
    Some(parts.join("\n"))
}

/// Compose the "As …, I want …, so that …." statement from the triple,
/// or `None` when any fragment is missing or blank.
pub fn build_user_story_statement(story: &StructuredUserStory) -> Option<String> {
    let as_a = story.as_a.as_deref()?.trim();
    let i_want = story.i_want.as_deref()?.trim();
    let so_that = story.so_that.as_deref()?.trim();
    if as_a.is_empty() || i_want.is_empty() || so_that.is_empty() {
        return None;
    }
    Some(format!("As {as_a}, I want {i_want}, so that {so_that}."))
}

/// Render a bullet-list section, or `None` when every item is blank.
pub fn render_bullet_section(heading: &str, items: &[String]) -> Option<String> {
    let rendered: Vec<String> = items
        .iter()
        .map(|item| strip_list_marker(item))
        .filter(|item| !item.is_empty())
        .map(|item| format!("- {item}"))
        .collect();
    if rendered.is_empty() {
        None
    } else {
        Some(format!("## {heading}\n\n{}", rendered.join("\n")))
    }
}

/// Render a `prefix-NNN` numbered requirement section (existing labels are
/// stripped and items renumbered contiguously), or `None` when empty.
pub fn render_numbered_requirement_section(
    heading: &str,
    prefix: &str,
    items: &[String],
) -> Option<String> {
    let rendered: Vec<String> = items
        .iter()
        .map(|item| strip_requirement_label(item))
        .filter(|item| !item.is_empty())
        .enumerate()
        .map(|(index, item)| format!("- **{prefix}-{number:03}**: {item}", number = index + 1))
        .collect();
    if rendered.is_empty() {
        None
    } else {
        Some(format!("## {heading}\n\n{}", rendered.join("\n")))
    }
}

/// Collapse a [`TextBlock`] into trimmed paragraphs, or `None` when blank.
pub fn normalize_text_block(text: &TextBlock) -> Option<String> {
    match text {
        TextBlock::Text(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        TextBlock::Paragraphs(values) => {
            let paragraphs: Vec<String> = values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if paragraphs.is_empty() {
                None
            } else {
                Some(paragraphs.join("\n\n"))
            }
        }
    }
}

/// Strip a leading `US-n:` label from a story title.
pub fn normalize_user_story_title(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(colon) = trimmed.find(':') {
        let head = trimmed[..colon].trim();
        if head.starts_with("US-") {
            return trimmed[colon + 1..].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Normalize a priority value to its `P`-prefixed uppercase form
/// (`"1"` → `P1`, `"p2"` → `P2`); blank input stays blank.
pub fn normalize_priority(priority: &str) -> String {
    let trimmed = priority.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with('P') {
        upper
    } else {
        format!("P{upper}")
    }
}

/// Strip a leading `- ` / `* ` / `1. ` list marker from `value`.
pub fn strip_list_marker(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return rest.trim().to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return rest.trim().to_string();
    }
    if let Some((head, rest)) = trimmed.split_once(". ") {
        if !head.is_empty() && head.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Strip an existing `FR-` / `NFR-` / `SC-` label (plain or bold) so the
/// renderer can renumber the requirement.
pub fn strip_requirement_label(value: &str) -> String {
    let without_list = strip_list_marker(value);
    let mut candidate = without_list.trim().trim_matches('*').trim().to_string();
    if let Some((head, rest)) = candidate.split_once(':') {
        let label = head.trim().trim_matches('*');
        if label.starts_with("FR-") || label.starts_with("NFR-") || label.starts_with("SC-") {
            return rest.trim().to_string();
        }
    }
    if candidate.starts_with("**") && candidate.contains("**:") {
        candidate = candidate.replacen("**", "", 1);
        candidate = candidate.replacen("**", "", 1);
        if let Some((head, rest)) = candidate.split_once(':') {
            let label = head.trim();
            if label.starts_with("FR-") || label.starts_with("NFR-") || label.starts_with("SC-") {
                return rest.trim().to_string();
            }
        }
    }
    without_list
}
