//! SPECs tab — browse and search local SPEC files

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

#[derive(Debug, Clone)]
pub struct SpecItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
}

#[derive(Debug, Clone)]
pub struct SpecsState {
    pub specs: Vec<SpecItem>,
    pub selected: usize,
    pub search_query: String,
    pub search_mode: bool,
    pub offset: usize,
}

impl SpecsState {
    pub fn new() -> Self {
        Self {
            specs: Vec::new(),
            selected: 0,
            search_query: String::new(),
            search_mode: false,
            offset: 0,
        }
    }

    fn visible_specs(&self) -> Vec<&SpecItem> {
        if self.search_query.is_empty() {
            self.specs.iter().collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.specs
                .iter()
                .filter(|s| {
                    s.id.to_lowercase().contains(&q)
                        || s.title.to_lowercase().contains(&q)
                        || s.status.to_lowercase().contains(&q)
                })
                .collect()
        }
    }
}

pub enum SpecsMessage {
    SelectPrev,
    SelectNext,
    ToggleSearch,
    SearchChar(char),
    SearchBackspace,
}

pub fn handle_key(state: &SpecsState, key: &KeyEvent) -> Option<SpecsMessage> {
    if state.search_mode {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => Some(SpecsMessage::ToggleSearch),
            KeyCode::Char(c) => Some(SpecsMessage::SearchChar(c)),
            KeyCode::Backspace => Some(SpecsMessage::SearchBackspace),
            _ => None,
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(SpecsMessage::SelectPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(SpecsMessage::SelectNext),
            KeyCode::Char('/') => Some(SpecsMessage::ToggleSearch),
            _ => None,
        }
    }
}

pub fn update(state: &mut SpecsState, msg: SpecsMessage) {
    match msg {
        SpecsMessage::SelectPrev => {
            state.selected = state.selected.saturating_sub(1);
        }
        SpecsMessage::SelectNext => {
            let max = state.visible_specs().len().saturating_sub(1);
            if state.selected < max {
                state.selected += 1;
            }
        }
        SpecsMessage::ToggleSearch => {
            state.search_mode = !state.search_mode;
            if !state.search_mode {
                state.search_query.clear();
                state.selected = 0;
            }
        }
        SpecsMessage::SearchChar(c) => {
            state.search_query.push(c);
            state.selected = 0;
        }
        SpecsMessage::SearchBackspace => {
            state.search_query.pop();
            state.selected = 0;
        }
    }
}

pub fn render(state: &SpecsState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 {
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),   // List
        Constraint::Length(if state.search_mode { 1 } else { 0 }),
    ])
    .split(area);

    // Header
    let count = state.visible_specs().len();
    let header = format!(" SPECs ({count})  [/] Search");
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    // List
    let visible = state.visible_specs();
    let list_area = layout[1];
    let max_rows = list_area.height as usize;

    // Scroll offset
    let offset = if state.selected >= max_rows {
        state.selected - max_rows + 1
    } else {
        0
    };

    for (i, spec) in visible.iter().skip(offset).take(max_rows).enumerate() {
        let y = list_area.y + i as u16;
        let is_selected = (i + offset) == state.selected;

        let marker = if is_selected { ">" } else { " " };
        let status_icon = match spec.status.as_str() {
            "open" => "\u{25CB}",    // ○
            "closed" => "\u{25CF}",  // ●
            _ => "\u{25CB}",
        };

        let line = format!(
            " {marker} {:<12} {status_icon} {:<10} {}",
            spec.id, spec.phase, spec.title
        );

        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };

        let span = Span::styled(line, style);
        buf.set_span(list_area.x, y, &span, list_area.width);
    }

    // Search bar
    if state.search_mode {
        let search_line = format!(" Search: {}_", state.search_query);
        let span = Span::styled(search_line, Style::default().fg(Color::Yellow));
        buf.set_span(layout[2].x, layout[2].y, &span, layout[2].width);
    }
}

/// Load SPEC items from specs/ directory
pub fn load_specs(repo_root: &std::path::Path) -> Vec<SpecItem> {
    let specs_dir = repo_root.join("specs");
    let mut items = Vec::new();

    let entries = match std::fs::read_dir(&specs_dir) {
        Ok(e) => e,
        Err(_) => return items,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if !name.starts_with("SPEC-") {
            continue;
        }

        let metadata_path = path.join("metadata.json");
        if !metadata_path.exists() {
            continue;
        }

        let meta = match std::fs::read_to_string(&metadata_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };

        items.push(SpecItem {
            id: meta["id"].as_str().unwrap_or(&name).to_string(),
            title: meta["title"].as_str().unwrap_or("").to_string(),
            status: meta["status"].as_str().unwrap_or("open").to_string(),
            phase: meta["phase"].as_str().unwrap_or("").to_string(),
        });
    }

    items.sort_by(|a, b| a.id.cmp(&b.id));
    items
}
