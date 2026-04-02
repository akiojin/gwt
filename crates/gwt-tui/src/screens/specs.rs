//! Specs management screen.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Detail sections available for a spec.
const DETAIL_SECTIONS: [&str; 5] = [
    "spec.md",
    "plan.md",
    "tasks.md",
    "research.md",
    "data-model.md",
];

/// A single spec entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecItem {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub status: String,
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
    /// Root directory used for spec file persistence.
    pub(crate) spec_root: Option<PathBuf>,
    /// Whether the detail section is being edited inline.
    pub(crate) detail_editing: bool,
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

fn spec_root_for_state(state: &SpecsState) -> Option<&Path> {
    state.spec_root.as_deref()
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
pub fn replace_markdown_section(
    content: &str,
    heading: &str,
    new_body: &str,
) -> Result<String, String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut start_idx = None;
    let mut heading_level = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let title = trimmed[level..].trim();
            if title == heading {
                start_idx = Some(idx);
                heading_level = level;
                break;
            }
        }
    }

    let start_idx = match start_idx {
        Some(idx) => idx,
        None => {
            let mut appended = String::new();
            if !content.is_empty() {
                appended.push_str(content.trim_end_matches('\n'));
                appended.push_str("\n\n");
            }
            appended.push_str(&format!(
                "{} {}\n",
                "#".repeat(heading_level.max(2)),
                heading
            ));
            if !new_body.is_empty() {
                appended.push_str(new_body.trim_end_matches('\n'));
                appended.push('\n');
            }
            return Ok(appended);
        }
    };

    let mut end_idx = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(start_idx + 1) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            end_idx = idx;
            break;
        }
    }

    let mut output = String::new();
    for line in &lines[..=start_idx] {
        output.push_str(line);
        output.push('\n');
    }
    output.push('\n');
    if !new_body.is_empty() {
        output.push_str(new_body.trim_end_matches('\n'));
        output.push('\n');
        output.push('\n');
    }
    if end_idx < lines.len() {
        for line in &lines[end_idx..] {
            output.push_str(line);
            output.push('\n');
        }
    }
    Ok(output)
}

/// Extract a markdown section body by heading.
pub fn extract_markdown_section(content: &str, heading: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut start_idx = None;

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let title = trimmed[level..].trim();
            if title == heading {
                start_idx = Some(idx);
                break;
            }
        }
    }

    let start_idx = start_idx?;
    let mut end_idx = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(start_idx + 1) {
        if line.trim_start().starts_with('#') {
            end_idx = idx;
            break;
        }
    }

    let body = lines[start_idx + 1..end_idx]
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    Some(body.trim().to_string())
}

/// Update a markdown artifact file on disk.
pub fn update_spec_markdown_section(
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
            let len = state.filtered_specs().len();
            super::move_up(&mut state.selected, len);
        }
        SpecsMessage::MoveDown => {
            let len = state.filtered_specs().len();
            super::move_down(&mut state.selected, len);
        }
        SpecsMessage::ToggleDetail => {
            if !state.filtered_specs().is_empty() {
                state.detail_view = !state.detail_view;
                if state.detail_view {
                    state.detail_section = 0;
                }
            }
        }
        SpecsMessage::NextSection => {
            if state.detail_view {
                state.detail_section = (state.detail_section + 1) % DETAIL_SECTIONS.len();
            }
        }
        SpecsMessage::PrevSection => {
            if state.detail_view {
                state.detail_section = if state.detail_section == 0 {
                    DETAIL_SECTIONS.len() - 1
                } else {
                    state.detail_section - 1
                };
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
                    state.edit_field = spec.phase.clone();
                    state.editing = true;
                    state.save_error = None;
                }
            }
        }
        SpecsMessage::SaveEdit => {
            if state.editing {
                let selected_id = state.selected_spec().map(|spec| spec.id.clone());
                if let Some(id) = selected_id {
                    if let Some(root) = spec_root_for_state(state) {
                        if let Err(err) =
                            update_spec_metadata_field(root, &id, "phase", &state.edit_field)
                        {
                            state.save_error = Some(err);
                            return;
                        }
                    }
                    if let Some(s) = state.specs.iter_mut().find(|s| s.id == id) {
                        s.phase = state.edit_field.clone();
                    }
                }
                state.save_error = None;
                state.editing = false;
                state.edit_field.clear();
            }
        }
        SpecsMessage::CancelEdit => {
            state.editing = false;
            state.edit_field.clear();
            state.save_error = None;
        }
        SpecsMessage::EditInput(ch) => {
            if state.editing {
                state.edit_field.push(ch);
            }
        }
        SpecsMessage::EditBackspace => {
            if state.editing {
                state.edit_field.pop();
            }
        }
        SpecsMessage::StartSectionEdit => {
            if state.detail_view && !state.detail_editing {
                if let Some(spec) = state.selected_spec() {
                    let section_name = DETAIL_SECTIONS[state.detail_section];
                    state.detail_edit_buffer = if let Some(root) = spec_root_for_state(state) {
                        fs::read_to_string(spec_markdown_path(root, &spec.id, section_name))
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
                        if let Err(err) = update_spec_markdown_section(
                            root,
                            &spec.id,
                            DETAIL_SECTIONS[state.detail_section],
                            &state.detail_edit_buffer,
                        ) {
                            state.save_error = Some(err);
                            return;
                        }
                    }
                    state.save_error = None;
                }
                state.detail_editing = false;
            }
        }
        SpecsMessage::CancelSectionEdit => {
            state.detail_editing = false;
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

    let block = Block::default().borders(Borders::ALL).title("Specs");
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

    let block = Block::default().borders(Borders::ALL);
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
            let block = Block::default().borders(Borders::ALL).title("Spec Detail");
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
            Constraint::Length(3), // Section tabs
            Constraint::Min(0),    // Section content
        ])
        .split(area);

    // Header section
    let header_text = if let Some(err) = &state.save_error {
        format!(
            " {} - {}\n Phase: {} | Status: {}\n Save error: {}\n Press Enter to go back | Tab/Shift+Tab: sections",
            spec.id, spec.title, spec.phase, spec.status, err
        )
    } else {
        format!(
            " {} - {}\n Phase: {} | Status: {}\n Press Enter to go back | Tab/Shift+Tab: sections",
            spec.id, spec.title, spec.phase, spec.status,
        )
    };
    let header_block = Block::default().borders(Borders::ALL).title("Spec Detail");
    let header = Paragraph::new(header_text)
        .block(header_block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(header, chunks[0]);

    // Section tabs
    let section_titles: Vec<Line> = DETAIL_SECTIONS.iter().map(|s| Line::from(*s)).collect();
    let section_tabs = ratatui::widgets::Tabs::new(section_titles)
        .block(Block::default().borders(Borders::ALL).title("Sections"))
        .select(state.detail_section)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(section_tabs, chunks[1]);

    // Section content placeholder
    let section_name = DETAIL_SECTIONS[state.detail_section];
    let content_text = if state.detail_editing {
        format!(
            "{}\n_\nEnter: save | Esc: cancel | Backspace: delete",
            state.detail_edit_buffer
        )
    } else if let Some(root) = spec_root_for_state(state) {
        read_spec_markdown_file(root, &spec.id, section_name).unwrap_or_else(|_| {
            format!(
                "Unable to load {} for {}\n\nPress 'e' to create or replace this file.",
                section_name, spec.id
            )
        })
    } else {
        format!(
            "Content of {} for {}\n\nSpec root is not configured.",
            section_name, spec.id,
        )
    };
    let content_block = Block::default()
        .borders(Borders::ALL)
        .title(if state.detail_editing {
            format!("Editing {}", section_name)
        } else {
            section_name.to_string()
        });
    let content = Paragraph::new(content_text)
        .block(content_block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));
    frame.render_widget(content, chunks[2]);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::fs;

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
        for _ in 0..4 {
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
        assert_eq!(state.detail_section, 4); // wraps to last
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
        state.detail_section = 3; // research.md
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
    fn detail_sections_constant_has_five_entries() {
        assert_eq!(DETAIL_SECTIONS.len(), 5);
        assert_eq!(DETAIL_SECTIONS[0], "spec.md");
        assert_eq!(DETAIL_SECTIONS[4], "data-model.md");
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
    fn edit_input_appends() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        update(&mut state, SpecsMessage::StartEdit);
        update(&mut state, SpecsMessage::EditInput('x'));
        assert_eq!(state.edit_field, "implementationx");
    }

    #[test]
    fn edit_input_ignored_when_not_editing() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        update(&mut state, SpecsMessage::EditInput('x'));
        assert!(state.edit_field.is_empty());
    }

    #[test]
    fn edit_backspace_removes() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        update(&mut state, SpecsMessage::StartEdit);
        update(&mut state, SpecsMessage::EditBackspace);
        assert_eq!(state.edit_field, "implementatio");
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
}
