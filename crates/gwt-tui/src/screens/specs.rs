//! Specs management screen.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::widgets;

/// Detail sections available for a spec.
const DETAIL_SECTIONS: [&str; 6] = [
    "spec.md",
    "plan.md",
    "tasks.md",
    "analysis.md",
    "research.md",
    "data-model.md",
];

const PHASE_OPTIONS: [&str; 4] = ["design", "planning", "implementation", "done"];
const STATUS_OPTIONS: [&str; 5] = ["draft", "open", "in-progress", "blocked", "done"];

/// A single spec entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecItem {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SpecEditTarget {
    #[default]
    Phase,
    Status,
}

/// State for the specs screen.
#[derive(Debug, Clone, Default)]
pub struct SpecsState {
    pub(crate) specs: Vec<SpecItem>,
    pub(crate) selected: usize,
    pub(crate) detail_view: bool,
    pub(crate) detail_section: usize,
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    /// Whether we are editing the phase field of the selected spec.
    pub(crate) editing: bool,
    /// Buffer for the phase field being edited.
    pub(crate) edit_field: String,
    /// Metadata field currently being edited.
    edit_target: SpecEditTarget,
    /// Available metadata values for the current edit session.
    edit_options: Vec<String>,
    /// Selected index within `edit_options`.
    edit_option_index: usize,
    /// Root directory used for spec file persistence.
    pub(crate) spec_root: Option<PathBuf>,
    /// Whether the detail section is being edited inline.
    pub(crate) detail_editing: bool,
    /// Active markdown heading index when viewing `spec.md`.
    detail_heading_index: usize,
    /// Heading currently being edited from `spec.md`.
    detail_edit_heading: Option<String>,
    /// Parsed section index currently being edited from `spec.md`.
    detail_edit_section_index: Option<usize>,
    /// Buffer for the detail section content being edited.
    pub(crate) detail_edit_buffer: String,
    /// Latest persistence error.
    pub(crate) save_error: Option<String>,
}

impl SpecsState {
    /// Return specs filtered by the current search query.
    pub fn filtered_specs(&self) -> Vec<&SpecItem> {
        let query_lower = self.search_query.to_lowercase();
        self.specs
            .iter()
            .filter(|s| {
                query_lower.is_empty()
                    || s.id.to_lowercase().contains(&query_lower)
                    || s.title.to_lowercase().contains(&query_lower)
                    || s.phase.to_lowercase().contains(&query_lower)
                    || s.status.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// Get the currently selected spec (from filtered list).
    pub fn selected_spec(&self) -> Option<&SpecItem> {
        let filtered = self.filtered_specs();
        filtered.get(self.selected).copied()
    }

    /// Clamp selected index to filtered length.
    fn clamp_selected(&mut self) {
        let len = self.filtered_specs().len();
        super::clamp_index(&mut self.selected, len);
    }
}

fn edit_target_label(target: SpecEditTarget) -> &'static str {
    match target {
        SpecEditTarget::Phase => "phase",
        SpecEditTarget::Status => "status",
    }
}

fn metadata_options(target: SpecEditTarget, current_value: &str) -> Vec<String> {
    let mut options = match target {
        SpecEditTarget::Phase => PHASE_OPTIONS
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
        SpecEditTarget::Status => STATUS_OPTIONS
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
    };

    if current_value.is_empty() {
        return options;
    }

    if let Some(index) = options
        .iter()
        .position(|option| option.eq_ignore_ascii_case(current_value))
    {
        options[index] = current_value.to_string();
    } else {
        options.insert(0, current_value.to_string());
    }

    options
}

fn start_metadata_edit(state: &mut SpecsState, target: SpecEditTarget, current_value: String) {
    let options = metadata_options(target, &current_value);
    let selected_index = options
        .iter()
        .position(|option| option == &current_value)
        .unwrap_or_default();
    state.edit_field = current_value;
    state.edit_target = target;
    state.edit_options = options;
    state.edit_option_index = selected_index;
    state.editing = true;
    state.save_error = None;
}

fn cycle_metadata_option(state: &mut SpecsState, direction: SpecsMessage) {
    if state.edit_options.is_empty() {
        return;
    }
    match direction {
        SpecsMessage::MoveUp => {
            super::move_up(&mut state.edit_option_index, state.edit_options.len())
        }
        SpecsMessage::MoveDown => {
            super::move_down(&mut state.edit_option_index, state.edit_options.len())
        }
        _ => return,
    }
    if let Some(selected) = state.edit_options.get(state.edit_option_index) {
        state.edit_field = selected.clone();
    }
}

fn spec_root_for_state(state: &SpecsState) -> Option<&Path> {
    state.spec_root.as_deref()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownSection {
    title: String,
    level: usize,
    start_line: usize,
    end_line: usize,
}

fn markdown_fence_marker(line: &str) -> Option<&'static str> {
    ["```", "~~~"]
        .into_iter()
        .find(|marker| line.trim_start().starts_with(marker))
}

fn markdown_heading_parts(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }

    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if trimmed.chars().nth(level) != Some(' ') {
        return None;
    }

    Some((level, trimmed[level + 1..].trim()))
}

fn markdown_sections(content: &str) -> Vec<MarkdownSection> {
    let lines: Vec<&str> = content.lines().collect();
    let mut sections = Vec::new();
    let mut current: Option<MarkdownSection> = None;
    let mut active_fence: Option<&'static str> = None;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();

        if let Some(marker) = active_fence {
            if trimmed.starts_with(marker) {
                active_fence = None;
            }
            continue;
        }

        if let Some(marker) = markdown_fence_marker(trimmed) {
            active_fence = Some(marker);
            continue;
        }

        let Some((level, title)) = markdown_heading_parts(trimmed) else {
            continue;
        };

        if current
            .as_ref()
            .is_some_and(|section| level <= section.level)
        {
            if let Some(mut section) = current.take() {
                section.end_line = idx;
                sections.push(section);
            }
        }

        if level == 2 {
            current = Some(MarkdownSection {
                title: title.to_string(),
                level,
                start_line: idx,
                end_line: lines.len(),
            });
        }
    }

    if let Some(mut section) = current {
        section.end_line = lines.len();
        sections.push(section);
    }

    sections
}

#[cfg(test)]
fn markdown_section_headings(content: &str) -> Vec<String> {
    markdown_sections(content)
        .into_iter()
        .map(|section| section.title)
        .collect()
}

fn current_spec_markdown_sections(state: &SpecsState) -> Vec<MarkdownSection> {
    if !state.detail_view || DETAIL_SECTIONS.get(state.detail_section) != Some(&"spec.md") {
        return Vec::new();
    }
    let Some(spec) = state.selected_spec() else {
        return Vec::new();
    };
    let Some(root) = spec_root_for_state(state) else {
        return Vec::new();
    };
    let Ok(content) = read_spec_markdown_file(root, &spec.id, "spec.md") else {
        return Vec::new();
    };
    markdown_sections(&content)
}

fn spec_dir(root: &Path, spec_id: &str) -> PathBuf {
    let spec_name = if spec_id.starts_with("SPEC-") {
        spec_id.to_string()
    } else {
        format!("SPEC-{spec_id}")
    };
    root.join("specs").join(spec_name)
}

fn spec_metadata_path(root: &Path, spec_id: &str) -> PathBuf {
    spec_dir(root, spec_id).join("metadata.json")
}

fn spec_markdown_path(root: &Path, spec_id: &str, file_name: &str) -> PathBuf {
    spec_dir(root, spec_id).join(file_name)
}

fn read_spec_markdown_file(root: &Path, spec_id: &str, file_name: &str) -> Result<String, String> {
    fs::read_to_string(spec_markdown_path(root, spec_id, file_name)).map_err(|e| e.to_string())
}

/// Update a metadata field in `metadata.json`.
pub fn update_spec_metadata_field(
    root: &Path,
    spec_id: &str,
    field_name: &str,
    field_value: &str,
) -> Result<(), String> {
    let path = spec_metadata_path(root, spec_id);
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| "metadata.json must contain a JSON object".to_string())?;
    obj.insert(
        field_name.to_string(),
        serde_json::Value::String(field_value.to_string()),
    );
    let serialized = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    fs::write(&path, serialized).map_err(|e| e.to_string())
}

/// Replace a markdown section by heading, preserving the rest of the file.
fn replace_markdown_section(
    content: &str,
    section: &MarkdownSection,
    new_body: &str,
) -> Result<String, String> {
    let lines: Vec<&str> = content.lines().collect();
    if section.start_line >= lines.len()
        || section.end_line > lines.len()
        || section.start_line >= section.end_line
    {
        return Err("selected markdown section is no longer present".to_string());
    }
    let Some((level, title)) = markdown_heading_parts(lines[section.start_line]) else {
        return Err("selected markdown section heading is invalid".to_string());
    };
    if level != section.level || title != section.title {
        return Err("selected markdown section no longer matches the file".to_string());
    }

    let mut output = String::new();
    for line in &lines[..=section.start_line] {
        output.push_str(line);
        output.push('\n');
    }
    output.push('\n');
    if !new_body.is_empty() {
        output.push_str(new_body.trim_end_matches('\n'));
        output.push('\n');
        output.push('\n');
    }
    if section.end_line < lines.len() {
        for line in &lines[section.end_line..] {
            output.push_str(line);
            output.push('\n');
        }
    }
    Ok(output)
}

/// Extract a markdown section body by heading.
fn extract_markdown_section(content: &str, section: &MarkdownSection) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if section.start_line >= lines.len()
        || section.end_line > lines.len()
        || section.start_line >= section.end_line
    {
        return None;
    }

    let body = lines[section.start_line + 1..section.end_line]
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    Some(body.trim().to_string())
}

/// Update a markdown artifact file on disk.
fn update_spec_markdown_section(
    root: &Path,
    spec_id: &str,
    file_name: &str,
    new_body: &str,
) -> Result<(), String> {
    let path = spec_markdown_path(root, spec_id, file_name);
    fs::write(&path, new_body).map_err(|e| e.to_string())
}

/// Messages specific to the specs screen.
#[derive(Debug, Clone)]
pub enum SpecsMessage {
    MoveUp,
    MoveDown,
    ToggleDetail,
    NextSection,
    PrevSection,
    SearchStart,
    SearchInput(char),
    SearchBackspace,
    SearchClear,
    Refresh,
    SetSpecs(Vec<SpecItem>),
    /// Launch an agent session for the selected spec (Shift+Enter).
    LaunchAgent,
    /// Start editing the phase of the selected spec.
    StartEdit,
    /// Start editing the status of the selected spec.
    StartStatusEdit,
    /// Start raw file editing for the active detail section.
    StartFileEdit,
    /// Save the current edit.
    SaveEdit,
    /// Cancel the current edit.
    CancelEdit,
    /// Append a character to the edit buffer.
    EditInput(char),
    /// Remove the last character from the edit buffer.
    EditBackspace,
    /// Start editing the active detail section.
    StartSectionEdit,
    /// Save the active detail section.
    SaveSectionEdit,
    /// Cancel the active detail section edit.
    CancelSectionEdit,
    /// Append a character to the detail section buffer.
    SectionEditInput(char),
    /// Remove the last character from the detail section buffer.
    SectionEditBackspace,
}

/// Update specs state in response to a message.
pub fn update(state: &mut SpecsState, msg: SpecsMessage) {
    match msg {
        SpecsMessage::MoveUp => {
            if state.editing {
                cycle_metadata_option(state, SpecsMessage::MoveUp);
            } else {
                let sections = current_spec_markdown_sections(state);
                if state.detail_view && !state.detail_editing && !sections.is_empty() {
                    super::move_up(&mut state.detail_heading_index, sections.len());
                } else {
                    let len = state.filtered_specs().len();
                    super::move_up(&mut state.selected, len);
                }
            }
        }
        SpecsMessage::MoveDown => {
            if state.editing {
                cycle_metadata_option(state, SpecsMessage::MoveDown);
            } else {
                let sections = current_spec_markdown_sections(state);
                if state.detail_view && !state.detail_editing && !sections.is_empty() {
                    super::move_down(&mut state.detail_heading_index, sections.len());
                } else {
                    let len = state.filtered_specs().len();
                    super::move_down(&mut state.selected, len);
                }
            }
        }
        SpecsMessage::ToggleDetail => {
            if !state.filtered_specs().is_empty() {
                state.detail_view = !state.detail_view;
                state.search_active = false;
                state.detail_heading_index = 0;
                state.detail_edit_heading = None;
                state.detail_edit_section_index = None;
                if state.detail_view {
                    state.detail_section = 0;
                }
            }
        }
        SpecsMessage::NextSection => {
            if state.detail_view {
                state.detail_section = (state.detail_section + 1) % DETAIL_SECTIONS.len();
                state.detail_heading_index = 0;
                state.detail_edit_heading = None;
                state.detail_edit_section_index = None;
            }
        }
        SpecsMessage::PrevSection => {
            if state.detail_view {
                state.detail_section = if state.detail_section == 0 {
                    DETAIL_SECTIONS.len() - 1
                } else {
                    state.detail_section - 1
                };
                state.detail_heading_index = 0;
                state.detail_edit_heading = None;
                state.detail_edit_section_index = None;
            }
        }
        SpecsMessage::SearchStart => {
            state.search_active = true;
        }
        SpecsMessage::SearchInput(ch) => {
            if state.search_active {
                state.search_query.push(ch);
                state.clamp_selected();
            }
        }
        SpecsMessage::SearchBackspace => {
            if state.search_active {
                state.search_query.pop();
                state.clamp_selected();
            }
        }
        SpecsMessage::SearchClear => {
            state.search_query.clear();
            state.search_active = false;
            state.clamp_selected();
        }
        SpecsMessage::Refresh => {
            // Signal to reload specs -- handled by caller
        }
        SpecsMessage::SetSpecs(specs) => {
            state.specs = specs;
            state.clamp_selected();
        }
        SpecsMessage::LaunchAgent => {
            // Signal handled by caller — selected spec context is read from state.
            // This is a no-op in the specs screen itself; the app layer reads
            // state.selected_spec() to prefill the wizard.
        }
        SpecsMessage::StartEdit => {
            if !state.editing {
                if let Some(spec) = state.selected_spec() {
                    start_metadata_edit(state, SpecEditTarget::Phase, spec.phase.clone());
                }
            }
        }
        SpecsMessage::StartStatusEdit => {
            if !state.editing {
                if let Some(spec) = state.selected_spec() {
                    start_metadata_edit(state, SpecEditTarget::Status, spec.status.clone());
                }
            }
        }
        SpecsMessage::SaveEdit => {
            if state.editing {
                let selected_id = state.selected_spec().map(|spec| spec.id.clone());
                if let Some(id) = selected_id {
                    let field_name = match state.edit_target {
                        SpecEditTarget::Phase => "phase",
                        SpecEditTarget::Status => "status",
                    };
                    if let Some(root) = spec_root_for_state(state) {
                        if let Err(err) =
                            update_spec_metadata_field(root, &id, field_name, &state.edit_field)
                        {
                            state.save_error = Some(err);
                            return;
                        }
                    }
                    if let Some(s) = state.specs.iter_mut().find(|s| s.id == id) {
                        match state.edit_target {
                            SpecEditTarget::Phase => s.phase = state.edit_field.clone(),
                            SpecEditTarget::Status => s.status = state.edit_field.clone(),
                        }
                    }
                }
                state.save_error = None;
                state.editing = false;
                state.edit_field.clear();
                state.edit_target = SpecEditTarget::Phase;
                state.edit_options.clear();
                state.edit_option_index = 0;
            }
        }
        SpecsMessage::CancelEdit => {
            state.editing = false;
            state.edit_field.clear();
            state.edit_target = SpecEditTarget::Phase;
            state.edit_options.clear();
            state.edit_option_index = 0;
            state.save_error = None;
        }
        SpecsMessage::EditInput(ch) => {
            if state.editing && state.edit_options.is_empty() {
                state.edit_field.push(ch);
            }
        }
        SpecsMessage::EditBackspace => {
            if state.editing && state.edit_options.is_empty() {
                state.edit_field.pop();
            }
        }
        SpecsMessage::StartSectionEdit => {
            if state.detail_view && !state.detail_editing {
                let selected_id = state.selected_spec().map(|spec| spec.id.clone());
                if let Some(id) = selected_id {
                    let section_name = DETAIL_SECTIONS[state.detail_section];
                    let sections = current_spec_markdown_sections(state);
                    let spec_root = state.spec_root.clone();
                    let (edit_heading, edit_section_index, edit_buffer) = if let Some(root) =
                        spec_root.as_deref()
                    {
                        if section_name == "spec.md" {
                            if let Some(section) = sections.get(state.detail_heading_index) {
                                let content =
                                    fs::read_to_string(spec_markdown_path(root, &id, section_name))
                                        .unwrap_or_default();
                                (
                                    Some(section.title.clone()),
                                    Some(state.detail_heading_index),
                                    extract_markdown_section(&content, section).unwrap_or_default(),
                                )
                            } else {
                                (
                                    None,
                                    None,
                                    fs::read_to_string(spec_markdown_path(root, &id, section_name))
                                        .unwrap_or_default(),
                                )
                            }
                        } else {
                            (
                                None,
                                None,
                                fs::read_to_string(spec_markdown_path(root, &id, section_name))
                                    .unwrap_or_default(),
                            )
                        }
                    } else {
                        (None, None, String::new())
                    };
                    state.detail_edit_heading = edit_heading;
                    state.detail_edit_section_index = edit_section_index;
                    state.detail_edit_buffer = edit_buffer;
                    state.detail_editing = true;
                    state.save_error = None;
                }
            }
        }
        SpecsMessage::StartFileEdit => {
            if state.detail_view && !state.detail_editing {
                let selected_id = state.selected_spec().map(|spec| spec.id.clone());
                if let Some(id) = selected_id {
                    let section_name = DETAIL_SECTIONS[state.detail_section];
                    let spec_root = state.spec_root.clone();
                    state.detail_edit_heading = None;
                    state.detail_edit_section_index = None;
                    state.detail_edit_buffer = if let Some(root) = spec_root.as_deref() {
                        fs::read_to_string(spec_markdown_path(root, &id, section_name))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    state.detail_editing = true;
                    state.save_error = None;
                }
            }
        }
        SpecsMessage::SaveSectionEdit => {
            if state.detail_editing {
                if let Some(spec) = state.selected_spec() {
                    if let Some(root) = spec_root_for_state(state) {
                        let file_name = DETAIL_SECTIONS[state.detail_section];
                        if let Some(section_index) = state.detail_edit_section_index {
                            let existing = match read_spec_markdown_file(root, &spec.id, file_name)
                            {
                                Ok(content) => content,
                                Err(err) => {
                                    state.save_error = Some(err);
                                    return;
                                }
                            };
                            let sections = markdown_sections(&existing);
                            let Some(section) = sections.get(section_index) else {
                                state.save_error = Some(
                                    "selected markdown section is no longer present".to_string(),
                                );
                                return;
                            };
                            if state.detail_edit_heading.as_deref() != Some(section.title.as_str())
                            {
                                state.save_error = Some(
                                    "selected markdown section no longer matches the file"
                                        .to_string(),
                                );
                                return;
                            }
                            let updated = match replace_markdown_section(
                                &existing,
                                section,
                                &state.detail_edit_buffer,
                            ) {
                                Ok(content) => content,
                                Err(err) => {
                                    state.save_error = Some(err);
                                    return;
                                }
                            };
                            if let Err(err) =
                                update_spec_markdown_section(root, &spec.id, file_name, &updated)
                            {
                                state.save_error = Some(err);
                                return;
                            }
                        } else if let Err(err) = update_spec_markdown_section(
                            root,
                            &spec.id,
                            file_name,
                            &state.detail_edit_buffer,
                        ) {
                            state.save_error = Some(err);
                            return;
                        }
                    }
                    state.save_error = None;
                }
                state.detail_editing = false;
                state.detail_edit_heading = None;
                state.detail_edit_section_index = None;
            }
        }
        SpecsMessage::CancelSectionEdit => {
            state.detail_editing = false;
            state.detail_edit_heading = None;
            state.detail_edit_section_index = None;
            state.detail_edit_buffer.clear();
            state.save_error = None;
        }
        SpecsMessage::SectionEditInput(ch) => {
            if state.detail_editing {
                state.detail_edit_buffer.push(ch);
            }
        }
        SpecsMessage::SectionEditBackspace => {
            if state.detail_editing {
                state.detail_edit_buffer.pop();
            }
        }
    }
}

/// Render the specs screen.
pub fn render(state: &SpecsState, frame: &mut Frame, area: Rect) {
    if state.detail_view {
        render_detail(state, frame, area);
    } else {
        render_list_view(state, frame, area);
    }
}

/// Render the list view with header and spec list.
fn render_list_view(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header: search bar
            Constraint::Min(0),    // Spec list
        ])
        .split(area);

    render_header(state, frame, chunks[0]);
    render_spec_list(state, frame, chunks[1]);
}

/// Render the header bar with search.
fn render_header(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let search_display = if state.search_active {
        format!(" Search: {}_", state.search_query)
    } else if !state.search_query.is_empty() {
        format!(" Search: {}", state.search_query)
    } else {
        " Press '/' to search".to_string()
    };

    let count = state.filtered_specs().len();
    let total = state.specs.len();
    let header_text = if let Some(err) = &state.save_error {
        format!(
            " Specs ({}/{})  |{}  | Save error: {}",
            count, total, search_display, err
        )
    } else {
        format!(" Specs ({}/{})  |{}", count, total, search_display)
    };

    let block = Block::default().title("Specs");
    let paragraph = Paragraph::new(header_text)
        .block(block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(paragraph, area);
}

/// Render the spec list.
fn render_spec_list(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_specs();

    if filtered.is_empty() {
        super::render_empty_list(frame, area, !state.specs.is_empty(), "specs");
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(idx, spec)| {
            let status_color = match spec.status.as_str() {
                "done" | "completed" => Color::Green,
                "in-progress" | "active" => Color::Yellow,
                "draft" | "planned" => Color::Cyan,
                "blocked" => Color::Red,
                _ => Color::DarkGray,
            };

            let style = super::list_item_style(idx == state.selected);

            let is_editing = idx == state.selected && state.editing;

            let phase_display = if is_editing {
                format!(" [{}_]", state.edit_field)
            } else {
                format!(" [{}]", spec.phase)
            };

            let phase_style = if is_editing {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Magenta)
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{:<10} ", spec.id),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(spec.title.clone(), style),
                Span::styled(phase_display, phase_style),
                Span::styled(
                    format!(" ({})", spec.status),
                    Style::default().fg(status_color),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default();
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Render the detail view for the selected spec.
fn render_detail(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let spec = match state.selected_spec() {
        Some(s) => s,
        None => {
            let block = Block::default().title("Spec Detail");
            let paragraph = Paragraph::new("No spec selected")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Spec header
            Constraint::Min(0),    // Section content (tabs in block title)
        ])
        .split(area);

    let section_name = DETAIL_SECTIONS[state.detail_section];
    let edit_hint = if section_name == "spec.md" {
        "Ctrl+e: edit section | E: edit file"
    } else {
        "Ctrl+e: edit file"
    };

    // Header section
    let header_text = if let Some(err) = &state.save_error {
        format!(
            " {} - {}\n Phase: {} | Status: {}\n Save error: {}\n Esc: back | ←→: sections | e: edit phase | s: edit status | {}",
            spec.id, spec.title, spec.phase, spec.status, err, edit_hint
        )
    } else {
        format!(
            " {} - {}\n Phase: {} | Status: {}\n Esc: back | ←→: sections | e: edit phase | s: edit status | {}",
            spec.id, spec.title, spec.phase, spec.status, edit_hint,
        )
    };
    let header_block = Block::default().title("Spec Detail");
    let header = Paragraph::new(header_text)
        .block(header_block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(header, chunks[0]);

    let sections = current_spec_markdown_sections(state);
    let selected_section = sections.get(state.detail_heading_index);
    let selected_heading = selected_section.map(|section| section.title.clone());
    let tab_title = if section_name == "spec.md" {
        match selected_heading.as_deref() {
            Some(heading) => format!("{section_name} :: {heading}"),
            None => section_name.to_string(),
        }
    } else {
        section_name.to_string()
    };

    if state.detail_editing {
        let content_text = match state.detail_edit_heading.as_deref() {
            Some(heading) => format!(
                "Editing section: {}\n{}_\nEnter: save | Esc: cancel | Backspace: delete",
                heading, state.detail_edit_buffer
            ),
            None => format!(
                "{}\n_\nEnter: save | Esc: cancel | Backspace: delete",
                state.detail_edit_buffer
            ),
        };
        let content_block = Block::default().title(tab_title);
        let content = Paragraph::new(content_text)
            .block(content_block)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::White));
        frame.render_widget(content, chunks[1]);
        return;
    }

    if state.editing {
        let options = if state.edit_options.is_empty() {
            vec![state.edit_field.clone()]
        } else {
            state.edit_options.clone()
        };
        let option_lines = options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let marker = if index == state.edit_option_index {
                    ">"
                } else {
                    " "
                };
                format!("{marker} {option}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let content_text = format!(
            "Select {}:\n↑↓: choose value | Enter: save | Esc: cancel\n\n{}",
            edit_target_label(state.edit_target),
            option_lines
        );
        let content_block = Block::default().title(tab_title);
        let content = Paragraph::new(content_text)
            .block(content_block)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::White));
        frame.render_widget(content, chunks[1]);
        return;
    }

    if let Some(root) = spec_root_for_state(state) {
        if let Ok(content) = read_spec_markdown_file(root, &spec.id, section_name) {
            if section_name == "spec.md" {
                if let Some(section) = selected_section {
                    let body = extract_markdown_section(&content, section)
                        .unwrap_or_else(|| "(empty section)".to_string());
                    let prelude = format!(
                        "Selected section: {}\n↑↓: choose section | Ctrl+e: edit selected section | E: edit file",
                        section.title
                    );
                    widgets::markdown::render_with_prelude(
                        &tab_title, &prelude, &body, frame, chunks[1],
                    );
                } else {
                    widgets::markdown::render_with_prelude(
                        &tab_title,
                        "Ctrl+e or E edits the entire file until a top-level section exists.",
                        &content,
                        frame,
                        chunks[1],
                    );
                }
            } else {
                widgets::markdown::render(&tab_title, &content, frame, chunks[1]);
            }
            return;
        }
    }

    let content_text = if let Some(root) = spec_root_for_state(state) {
        read_spec_markdown_file(root, &spec.id, section_name)
            .map(|content| {
                if section_name == "spec.md" {
                    if let Some(section) = selected_section {
                        let body = extract_markdown_section(&content, section)
                            .unwrap_or_else(|| "(empty section)".to_string());
                        format!(
                            "Selected section: {}\n↑↓: choose section | Ctrl+e: edit selected section | E: edit file\n\n{}",
                            section.title, body
                        )
                    } else {
                        format!(
                            "{}\n\nCtrl+e or E edits the entire file until a top-level section exists.",
                            content
                        )
                    }
                } else {
                    content
                }
            })
            .unwrap_or_else(|_| {
                format!(
                    "Unable to load {} for {}\n\nPress Ctrl+e or E to create or replace this file.",
                    section_name, spec.id
                )
            })
    } else {
        format!(
            "Content of {} for {}\n\nSpec root is not configured.",
            section_name, spec.id,
        )
    };
    let content_block = Block::default().title(tab_title);
    let content = Paragraph::new(content_text)
        .block(content_block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));
    frame.render_widget(content, chunks[1]);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::fs;

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn sample_specs() -> Vec<SpecItem> {
        vec![
            SpecItem {
                id: "SPEC-1".to_string(),
                title: "Add worktree support".to_string(),
                phase: "implementation".to_string(),
                status: "in-progress".to_string(),
            },
            SpecItem {
                id: "SPEC-2".to_string(),
                title: "Agent orchestration".to_string(),
                phase: "planning".to_string(),
                status: "draft".to_string(),
            },
            SpecItem {
                id: "SPEC-3".to_string(),
                title: "Voice commands".to_string(),
                phase: "completed".to_string(),
                status: "done".to_string(),
            },
            SpecItem {
                id: "SPEC-10".to_string(),
                title: "Settings UI".to_string(),
                phase: "design".to_string(),
                status: "blocked".to_string(),
            },
        ]
    }

    fn write_spec_fixture(root: &std::path::Path, spec_id: &str) {
        let spec_dir = super::spec_dir(root, spec_id);
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(
            spec_dir.join("metadata.json"),
            serde_json::json!({
                "id": spec_id,
                "title": "Fixture",
                "phase": "draft",
                "status": "open"
            })
            .to_string(),
        )
        .unwrap();
        fs::write(spec_dir.join("spec.md"), "# spec.md\n\noriginal\n").unwrap();
        fs::write(spec_dir.join("plan.md"), "# plan.md\n\nplan body\n").unwrap();
        fs::write(spec_dir.join("tasks.md"), "# tasks.md\n\ntasks body\n").unwrap();
        fs::write(
            spec_dir.join("analysis.md"),
            "# analysis.md\n\nanalysis body\n",
        )
        .unwrap();
        fs::write(
            spec_dir.join("research.md"),
            "# research.md\n\nresearch body\n",
        )
        .unwrap();
        fs::write(
            spec_dir.join("data-model.md"),
            "# data-model.md\n\ndata model body\n",
        )
        .unwrap();
    }

    fn write_sectioned_spec_fixture(root: &std::path::Path, spec_id: &str) {
        write_spec_fixture(root, spec_id);
        let spec_dir = super::spec_dir(root, spec_id);
        fs::write(
            spec_dir.join("spec.md"),
            "# SPEC fixture\n\n## Background\n\nbackground body\n\n## User Stories\n\nstories body\n",
        )
        .unwrap();
    }

    fn write_duplicate_section_fixture(root: &std::path::Path, spec_id: &str) {
        write_spec_fixture(root, spec_id);
        let spec_dir = super::spec_dir(root, spec_id);
        fs::write(
            spec_dir.join("spec.md"),
            "# SPEC fixture\n\n## Duplicate\n\nfirst body\n\n## Duplicate\n\nsecond body\n",
        )
        .unwrap();
    }

    #[test]
    fn default_state() {
        let state = SpecsState::default();
        assert!(state.specs.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.detail_view);
        assert_eq!(state.detail_section, 0);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();

        update(&mut state, SpecsMessage::MoveDown);
        assert_eq!(state.selected, 1);

        for _ in 0..3 {
            update(&mut state, SpecsMessage::MoveDown);
        }
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();

        update(&mut state, SpecsMessage::MoveUp);
        assert_eq!(state.selected, 3); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, SpecsMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn toggle_detail_flips() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        assert!(!state.detail_view);

        update(&mut state, SpecsMessage::ToggleDetail);
        assert!(state.detail_view);
        assert_eq!(state.detail_section, 0); // reset on open

        update(&mut state, SpecsMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn toggle_detail_noop_on_empty() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn next_section_cycles() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        assert_eq!(state.detail_section, 0);

        update(&mut state, SpecsMessage::NextSection);
        assert_eq!(state.detail_section, 1);

        // Cycle through all sections
        for _ in 0..5 {
            update(&mut state, SpecsMessage::NextSection);
        }
        assert_eq!(state.detail_section, 0); // wraps
    }

    #[test]
    fn prev_section_cycles() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        assert_eq!(state.detail_section, 0);

        update(&mut state, SpecsMessage::PrevSection);
        assert_eq!(state.detail_section, 5); // wraps to last
    }

    #[test]
    fn section_navigation_noop_outside_detail() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        assert!(!state.detail_view);

        update(&mut state, SpecsMessage::NextSection);
        assert_eq!(state.detail_section, 0);

        update(&mut state, SpecsMessage::PrevSection);
        assert_eq!(state.detail_section, 0);
    }

    #[test]
    fn search_start_activates() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::SearchStart);
        assert!(state.search_active);
    }

    #[test]
    fn search_input_appends() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::SearchStart);
        update(&mut state, SpecsMessage::SearchInput('w'));
        update(&mut state, SpecsMessage::SearchInput('o'));
        assert_eq!(state.search_query, "wo");
    }

    #[test]
    fn search_input_ignored_when_inactive() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::SearchInput('x'));
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn search_clear_resets() {
        let mut state = SpecsState::default();
        state.search_active = true;
        state.search_query = "test".to_string();

        update(&mut state, SpecsMessage::SearchClear);
        assert!(!state.search_active);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn set_specs_populates() {
        let mut state = SpecsState::default();
        state.selected = 99;
        update(&mut state, SpecsMessage::SetSpecs(sample_specs()));
        assert_eq!(state.specs.len(), 4);
        assert_eq!(state.selected, 3); // clamped
    }

    #[test]
    fn filtered_specs_respects_search_by_title() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "worktree".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-1");
    }

    #[test]
    fn filtered_specs_search_by_id() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "SPEC-10".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-10");
    }

    #[test]
    fn filtered_specs_search_by_phase() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "planning".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-2");
    }

    #[test]
    fn filtered_specs_search_by_status() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "blocked".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-10");
    }

    #[test]
    fn filtered_specs_case_insensitive() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "WORKTREE".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-1");
    }

    #[test]
    fn filtered_specs_empty_query_returns_all() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = String::new();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 4);
    }

    #[test]
    fn selected_spec_returns_correct() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 2;
        let spec = state.selected_spec().unwrap();
        assert_eq!(spec.id, "SPEC-3");
    }

    #[test]
    fn render_list_does_not_panic() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Specs"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = SpecsState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_detail_does_not_panic() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_detail_with_section_does_not_panic() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        state.detail_section = 4; // research.md
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_detail_status_edit_shows_prompt_and_buffer() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        state.editing = true;
        state.edit_target = SpecEditTarget::Status;
        state.edit_field = "in-progress".to_string();
        state.edit_options = vec![
            "draft".to_string(),
            "open".to_string(),
            "in-progress".to_string(),
            "blocked".to_string(),
            "done".to_string(),
        ];
        state.edit_option_index = 2;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Select status:"));
        assert!(text.contains("↑↓: choose value"));
        assert!(text.contains("> in-progress"));
    }

    #[test]
    fn search_clamps_selected_when_filtering() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 3; // last item

        update(&mut state, SpecsMessage::SearchStart);
        update(&mut state, SpecsMessage::SearchInput('v'));
        update(&mut state, SpecsMessage::SearchInput('o'));
        update(&mut state, SpecsMessage::SearchInput('i'));
        update(&mut state, SpecsMessage::SearchInput('c'));
        update(&mut state, SpecsMessage::SearchInput('e'));
        // "voice" matches "Voice commands"
        let filtered = state.filtered_specs();
        assert!(state.selected < filtered.len().max(1));
    }

    #[test]
    fn detail_sections_constant_includes_analysis_md() {
        assert_eq!(DETAIL_SECTIONS.len(), 6);
        assert_eq!(DETAIL_SECTIONS[0], "spec.md");
        assert_eq!(DETAIL_SECTIONS[3], "analysis.md");
        assert_eq!(DETAIL_SECTIONS.last().copied(), Some("data-model.md"));
    }

    #[test]
    fn start_section_edit_reads_analysis_markdown_file() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-102");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-102".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 3; // analysis.md

        update(&mut state, SpecsMessage::StartSectionEdit);

        assert!(state.detail_editing);
        assert!(state.detail_edit_buffer.contains("analysis body"));
    }

    #[test]
    fn move_down_in_spec_detail_cycles_markdown_headings() {
        let dir = tempfile::tempdir().unwrap();
        write_sectioned_spec_fixture(dir.path(), "SPEC-103");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-103".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;

        update(&mut state, SpecsMessage::MoveDown);

        assert_eq!(state.selected, 0);
        assert_eq!(state.detail_heading_index, 1);
    }

    #[test]
    fn start_section_edit_on_spec_md_reads_selected_heading_body() {
        let dir = tempfile::tempdir().unwrap();
        write_sectioned_spec_fixture(dir.path(), "SPEC-104");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-104".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;
        state.detail_heading_index = 1;

        update(&mut state, SpecsMessage::StartSectionEdit);

        assert!(state.detail_editing);
        assert_eq!(state.detail_edit_heading.as_deref(), Some("User Stories"));
        assert_eq!(state.detail_edit_buffer, "stories body");
    }

    #[test]
    fn update_spec_metadata_field_persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-99");

        update_spec_metadata_field(dir.path(), "SPEC-99", "phase", "implementation").unwrap();

        let metadata =
            fs::read_to_string(super::spec_metadata_path(dir.path(), "SPEC-99")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(parsed["phase"], "implementation");
    }

    #[test]
    fn save_edit_updates_metadata_file() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-100");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-100".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.selected = 0;
        update(&mut state, SpecsMessage::StartEdit);
        state.edit_field = "implementation".to_string();
        update(&mut state, SpecsMessage::SaveEdit);

        let metadata =
            fs::read_to_string(super::spec_metadata_path(dir.path(), "SPEC-100")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(parsed["phase"], "implementation");
        assert_eq!(state.specs[0].phase, "implementation");
        assert!(state.save_error.is_none());
    }

    #[test]
    fn save_section_edit_updates_markdown_file() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-101");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-101".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;
        update(&mut state, SpecsMessage::StartSectionEdit);
        assert!(state.detail_editing);
        assert!(state.detail_edit_buffer.contains("original"));

        state.detail_edit_buffer = "# spec.md\n\nupdated body\n".to_string();
        update(&mut state, SpecsMessage::SaveSectionEdit);

        let body = fs::read_to_string(super::spec_markdown_path(dir.path(), "SPEC-101", "spec.md"))
            .unwrap();
        assert_eq!(body, "# spec.md\n\nupdated body\n");
        assert!(!state.detail_editing);
        assert!(state.save_error.is_none());
    }

    #[test]
    fn save_section_edit_replaces_only_selected_heading() {
        let dir = tempfile::tempdir().unwrap();
        write_sectioned_spec_fixture(dir.path(), "SPEC-105");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-105".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;
        state.detail_heading_index = 1;

        update(&mut state, SpecsMessage::StartSectionEdit);
        state.detail_edit_buffer = "updated stories".to_string();
        update(&mut state, SpecsMessage::SaveSectionEdit);

        let body = fs::read_to_string(super::spec_markdown_path(dir.path(), "SPEC-105", "spec.md"))
            .unwrap();
        assert!(body.contains("## Background\n\nbackground body"));
        assert!(body.contains("## User Stories\n\nupdated stories"));
        assert!(!body.contains("stories body"));
        assert!(!state.detail_editing);
        assert!(state.save_error.is_none());
    }

    #[test]
    fn save_section_edit_uses_selected_duplicate_heading_index() {
        let dir = tempfile::tempdir().unwrap();
        write_duplicate_section_fixture(dir.path(), "SPEC-105A");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-105A".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;
        state.detail_heading_index = 1;

        update(&mut state, SpecsMessage::StartSectionEdit);
        state.detail_edit_buffer = "updated second body".to_string();
        update(&mut state, SpecsMessage::SaveSectionEdit);

        let body = fs::read_to_string(super::spec_markdown_path(
            dir.path(),
            "SPEC-105A",
            "spec.md",
        ))
        .unwrap();
        assert!(body.contains("## Duplicate\n\nfirst body"));
        assert!(body.contains("## Duplicate\n\nupdated second body"));
        assert!(
            !body.contains("## Duplicate\n\nupdated second body\n\n## Duplicate\n\nsecond body")
        );
        assert!(!state.detail_editing);
        assert!(state.save_error.is_none());
    }

    #[test]
    fn save_section_edit_errors_when_selected_section_disappears() {
        let dir = tempfile::tempdir().unwrap();
        write_sectioned_spec_fixture(dir.path(), "SPEC-105B");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-105B".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;
        state.detail_heading_index = 1;

        update(&mut state, SpecsMessage::StartSectionEdit);
        fs::write(
            super::spec_markdown_path(dir.path(), "SPEC-105B", "spec.md"),
            "# SPEC fixture\n\n## Background\n\nbackground body\n",
        )
        .unwrap();
        state.detail_edit_buffer = "updated stories".to_string();

        update(&mut state, SpecsMessage::SaveSectionEdit);

        let body = fs::read_to_string(super::spec_markdown_path(
            dir.path(),
            "SPEC-105B",
            "spec.md",
        ))
        .unwrap();
        assert!(state.detail_editing);
        assert!(state.save_error.is_some());
        assert!(!body.contains("updated stories"));
        assert!(!body.contains("## User Stories"));
    }

    #[test]
    fn extract_markdown_section_preserves_nested_headings() {
        let content =
            "# SPEC\n\n## User Stories\n\n### US-1\n\nstory body\n\n## Success Criteria\n\nsuccess\n";
        let sections = markdown_sections(content);

        let extracted = extract_markdown_section(content, &sections[0]).expect("section");

        assert!(extracted.contains("### US-1"));
        assert!(extracted.contains("story body"));
        assert!(!extracted.contains("## Success Criteria"));
    }

    #[test]
    fn replace_markdown_section_preserves_nested_headings() {
        let content =
            "# SPEC\n\n## User Stories\n\n### US-1\n\nstory body\n\n## Success Criteria\n\nsuccess\n";
        let sections = markdown_sections(content);

        let replaced =
            replace_markdown_section(content, &sections[0], "updated body").expect("replace");

        assert!(replaced.contains("## User Stories\n\nupdated body"));
        assert!(replaced.contains("## Success Criteria\n\nsuccess"));
        assert!(!replaced.contains("### US-1"));
    }

    #[test]
    fn markdown_section_headings_ignores_fenced_code_blocks() {
        let content = "# SPEC\n\n```md\n## Not a section\n```\n\n## Real Section\n\nbody\n";

        let headings = markdown_section_headings(content);

        assert_eq!(headings, vec!["Real Section".to_string()]);
    }

    #[test]
    fn render_detail_spec_md_shows_selected_heading_hint() {
        let dir = tempfile::tempdir().unwrap();
        write_sectioned_spec_fixture(dir.path(), "SPEC-106");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-106".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;
        state.detail_heading_index = 1;

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Selected section: User Stories"));
        assert!(text.contains("↑↓: choose section"));
    }

    #[test]
    fn render_detail_analysis_md_uses_markdown_bullet_rendering() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-106A");
        fs::write(
            super::spec_markdown_path(dir.path(), "SPEC-106A", "analysis.md"),
            "## Findings\n\n- first issue\n",
        )
        .unwrap();

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-106A".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 3;

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("• first issue"));
        assert!(!text.contains("- first issue"));
    }

    #[test]
    fn render_detail_spec_md_section_uses_markdown_bullet_rendering() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-106B");
        fs::write(
            super::spec_markdown_path(dir.path(), "SPEC-106B", "spec.md"),
            "# SPEC fixture\n\n## User Stories\n\n### Story\n\n- bullet item\n",
        )
        .unwrap();

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-106B".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.detail_view = true;
        state.detail_section = 0;

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("• bullet item"));
        assert!(!text.contains("- bullet item"));
    }

    // ---- LaunchAgent tests ----

    #[test]
    fn launch_agent_noop_on_empty() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::LaunchAgent);
        // No panic, no state change
        assert!(state.specs.is_empty());
    }

    #[test]
    fn launch_agent_preserves_selection() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 1;
        update(&mut state, SpecsMessage::LaunchAgent);
        assert_eq!(state.selected, 1);
        let spec = state.selected_spec().unwrap();
        assert_eq!(spec.id, "SPEC-2");
    }

    #[test]
    fn launch_agent_returns_context_from_selected() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 2;
        update(&mut state, SpecsMessage::LaunchAgent);
        let spec = state.selected_spec().unwrap();
        assert_eq!(spec.id, "SPEC-3");
        assert_eq!(spec.title, "Voice commands");
    }

    #[test]
    fn launch_agent_respects_search_filter() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "agent".to_string();
        state.selected = 0;
        update(&mut state, SpecsMessage::LaunchAgent);
        let spec = state.selected_spec().unwrap();
        assert_eq!(spec.id, "SPEC-2"); // "Agent orchestration" matches
    }

    #[test]
    fn launch_agent_with_filtered_empty_is_safe() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "nonexistent_query_xyz".to_string();
        update(&mut state, SpecsMessage::LaunchAgent);
        assert!(state.selected_spec().is_none());
    }

    // ---- Edit tests ----

    #[test]
    fn start_edit_enters_edit_mode() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 0;
        update(&mut state, SpecsMessage::StartEdit);
        assert!(state.editing);
        assert_eq!(state.edit_field, "implementation");
    }

    #[test]
    fn start_edit_noop_on_empty() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::StartEdit);
        assert!(!state.editing);
    }

    #[test]
    fn edit_input_is_ignored_for_selection_menu_metadata_edit() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        update(&mut state, SpecsMessage::StartEdit);
        update(&mut state, SpecsMessage::EditInput('x'));
        assert_eq!(state.edit_field, "implementation");
    }

    #[test]
    fn edit_input_ignored_when_not_editing() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        update(&mut state, SpecsMessage::EditInput('x'));
        assert!(state.edit_field.is_empty());
    }

    #[test]
    fn move_down_while_editing_phase_cycles_selection_menu_value() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 0;
        update(&mut state, SpecsMessage::StartEdit);

        update(&mut state, SpecsMessage::MoveDown);

        assert!(state.editing);
        assert_eq!(state.selected, 0);
        assert_eq!(state.edit_field, "done");
    }

    #[test]
    fn move_down_while_editing_status_cycles_selection_menu_value() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 0;
        update(&mut state, SpecsMessage::StartStatusEdit);

        update(&mut state, SpecsMessage::MoveDown);

        assert!(state.editing);
        assert_eq!(state.selected, 0);
        assert_eq!(state.edit_field, "blocked");
    }

    #[test]
    fn edit_backspace_is_ignored_for_selection_menu_metadata_edit() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        update(&mut state, SpecsMessage::StartEdit);
        update(&mut state, SpecsMessage::EditBackspace);
        assert_eq!(state.edit_field, "implementation");
    }

    #[test]
    fn edit_backspace_on_empty_is_noop() {
        let mut state = SpecsState::default();
        state.editing = true;
        state.edit_field.clear();
        update(&mut state, SpecsMessage::EditBackspace);
        assert!(state.edit_field.is_empty());
    }

    #[test]
    fn save_edit_updates_phase() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 0;
        update(&mut state, SpecsMessage::StartEdit);
        state.edit_field = "done".to_string();
        update(&mut state, SpecsMessage::SaveEdit);
        assert!(!state.editing);
        assert_eq!(state.specs[0].phase, "done");
    }

    #[test]
    fn save_edit_updates_status() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-102");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-102".to_string(),
            title: "Fixture".to_string(),
            phase: "draft".to_string(),
            status: "open".to_string(),
        }];
        state.selected = 0;
        update(&mut state, SpecsMessage::StartStatusEdit);
        state.edit_field = "in-progress".to_string();
        update(&mut state, SpecsMessage::SaveEdit);

        let metadata =
            fs::read_to_string(super::spec_metadata_path(dir.path(), "SPEC-102")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(parsed["status"], "in-progress");
        assert_eq!(state.specs[0].status, "in-progress");
        assert!(state.save_error.is_none());
    }

    #[test]
    fn save_edit_updates_phase_from_selection_menu_value() {
        let dir = tempfile::tempdir().unwrap();
        write_spec_fixture(dir.path(), "SPEC-103");

        let mut state = SpecsState::default();
        state.spec_root = Some(dir.path().to_path_buf());
        state.specs = vec![SpecItem {
            id: "SPEC-103".to_string(),
            title: "Fixture".to_string(),
            phase: "implementation".to_string(),
            status: "open".to_string(),
        }];
        state.selected = 0;
        update(&mut state, SpecsMessage::StartEdit);
        update(&mut state, SpecsMessage::MoveDown);
        update(&mut state, SpecsMessage::SaveEdit);

        let metadata =
            fs::read_to_string(super::spec_metadata_path(dir.path(), "SPEC-103")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(parsed["phase"], "done");
        assert_eq!(state.specs[0].phase, "done");
        assert!(state.save_error.is_none());
    }

    #[test]
    fn cancel_edit_discards() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 0;
        update(&mut state, SpecsMessage::StartEdit);
        state.edit_field = "changed".to_string();
        update(&mut state, SpecsMessage::CancelEdit);
        assert!(!state.editing);
        assert_eq!(state.specs[0].phase, "implementation"); // unchanged
    }

    #[test]
    fn save_edit_noop_when_not_editing() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        let original = state.specs[0].phase.clone();
        update(&mut state, SpecsMessage::SaveEdit);
        assert_eq!(state.specs[0].phase, original);
    }

    #[test]
    fn render_editing_does_not_panic() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.editing = true;
        state.edit_field = "testing".to_string();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_detail_phase_edit_shows_selection_menu_hint() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 0;
        state.detail_view = true;
        update(&mut state, SpecsMessage::StartEdit);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());

        assert!(text.contains("Select phase:"));
        assert!(text.contains("↑↓: choose value"));
        assert!(text.contains("> implementation"));
    }
}
