//! Structured spec JSON model + render / merge / split helpers
//! (SPEC-1942 SC-027 split for `cli::issue_spec`).
//!
//! Hosts the `StructuredSpecInput` types parsed from `--edit spec --json`
//! payloads, the renderer that reconstructs canonical Markdown sections,
//! and the merge/split helpers that allow incremental updates of an
//! existing spec body.

use std::collections::BTreeMap;

use gwt_github::{client::ApiError, SpecOpsError};
use serde::Deserialize;

use crate::cli::CliEnv;

fn io_as_api_error(err: std::io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

pub(super) const CANONICAL_SECTION_HEADINGS: [&str; 6] = [
    "Background",
    "User Stories",
    "Edge Cases",
    "Functional Requirements",
    "Non-Functional Requirements",
    "Success Criteria",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum TextBlock {
    Text(String),
    Paragraphs(Vec<String>),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct StructuredSpecInput {
    #[serde(default)]
    pub background: Option<TextBlock>,
    #[serde(default)]
    pub user_stories: Option<Vec<StructuredUserStory>>,
    #[serde(default)]
    pub edge_cases: Option<Vec<String>>,
    #[serde(default)]
    pub functional_requirements: Option<Vec<String>>,
    #[serde(default)]
    pub non_functional_requirements: Option<Vec<String>>,
    #[serde(default)]
    pub success_criteria: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct StructuredUserStory {
    pub title: String,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub statement: Option<String>,
    #[serde(default)]
    pub as_a: Option<String>,
    #[serde(default)]
    pub i_want: Option<String>,
    #[serde(default)]
    pub so_that: Option<String>,
    #[serde(default, alias = "acceptance")]
    pub acceptance_scenarios: Vec<String>,
}

pub(super) fn read_cli_input<E: CliEnv>(
    env: &mut E,
    file: Option<&str>,
) -> Result<String, SpecOpsError> {
    match file {
        None | Some("-") => env.read_stdin().map_err(io_as_api_error),
        Some(path) => env.read_file(path).map_err(io_as_api_error),
    }
}

pub(super) fn parse_structured_spec_json(raw: &str) -> Result<StructuredSpecInput, SpecOpsError> {
    serde_json::from_str(raw).map_err(|err| {
        SpecOpsError::from(ApiError::Unexpected(format!("invalid spec json: {err}")))
    })
}

pub(super) fn render_structured_spec(title: &str, structured: &StructuredSpecInput) -> String {
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

pub(super) fn merge_structured_spec(existing: &str, patch: &StructuredSpecInput) -> String {
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

pub(super) fn rendered_sections_for_patch(
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

pub(super) fn split_structured_spec(
    existing: &str,
) -> (String, BTreeMap<String, String>, Vec<String>) {
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

pub(super) fn extract_document_title(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn normalize_spec_heading_from_title(title: &str) -> String {
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

pub(super) fn render_background_section(background: &TextBlock) -> Option<String> {
    let content = normalize_text_block(background)?;
    Some(format!("## Background\n\n{content}"))
}

pub(super) fn render_user_stories_section(user_stories: &[StructuredUserStory]) -> Option<String> {
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

pub(super) fn render_user_story(index: usize, story: &StructuredUserStory) -> Option<String> {
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

pub(super) fn build_user_story_statement(story: &StructuredUserStory) -> Option<String> {
    let as_a = story.as_a.as_deref()?.trim();
    let i_want = story.i_want.as_deref()?.trim();
    let so_that = story.so_that.as_deref()?.trim();
    if as_a.is_empty() || i_want.is_empty() || so_that.is_empty() {
        return None;
    }
    Some(format!("As {as_a}, I want {i_want}, so that {so_that}."))
}

pub(super) fn render_bullet_section(heading: &str, items: &[String]) -> Option<String> {
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

pub(super) fn render_numbered_requirement_section(
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

pub(super) fn normalize_text_block(text: &TextBlock) -> Option<String> {
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

pub(super) fn normalize_user_story_title(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(colon) = trimmed.find(':') {
        let head = trimmed[..colon].trim();
        if head.starts_with("US-") {
            return trimmed[colon + 1..].trim().to_string();
        }
    }
    trimmed.to_string()
}

pub(super) fn normalize_priority(priority: &str) -> String {
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

pub(super) fn strip_list_marker(value: &str) -> String {
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

pub(super) fn strip_requirement_label(value: &str) -> String {
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
