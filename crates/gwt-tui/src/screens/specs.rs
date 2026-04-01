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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::prelude::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn sample_specs() -> Vec<SpecItem> {
        vec![
            SpecItem {
                id: "SPEC-1".into(),
                title: "Alpha feature".into(),
                status: "open".into(),
                phase: "planning".into(),
            },
            SpecItem {
                id: "SPEC-2".into(),
                title: "Beta bugfix".into(),
                status: "closed".into(),
                phase: "done".into(),
            },
            SpecItem {
                id: "SPEC-3".into(),
                title: "Gamma refactor".into(),
                status: "open".into(),
                phase: "implementation".into(),
            },
        ]
    }

    // -- SpecsState::new() --

    #[test]
    fn new_state_is_empty() {
        let s = SpecsState::new();
        assert!(s.specs.is_empty());
        assert_eq!(s.selected, 0);
        assert!(s.search_query.is_empty());
        assert!(!s.search_mode);
        assert_eq!(s.offset, 0);
    }

    // -- visible_specs() --

    #[test]
    fn visible_specs_returns_all_when_query_empty() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        assert_eq!(s.visible_specs().len(), 3);
    }

    #[test]
    fn visible_specs_filters_by_id() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.search_query = "SPEC-2".into();
        let v = s.visible_specs();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "SPEC-2");
    }

    #[test]
    fn visible_specs_filters_by_title_case_insensitive() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.search_query = "alpha".into();
        let v = s.visible_specs();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].title, "Alpha feature");
    }

    #[test]
    fn visible_specs_filters_by_status() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.search_query = "closed".into();
        let v = s.visible_specs();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "SPEC-2");
    }

    #[test]
    fn visible_specs_returns_empty_on_no_match() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.search_query = "nonexistent".into();
        assert!(s.visible_specs().is_empty());
    }

    // -- handle_key() normal mode --

    #[test]
    fn handle_key_up_returns_select_prev() {
        let s = SpecsState::new();
        assert!(matches!(handle_key(&s, &key(KeyCode::Up)), Some(SpecsMessage::SelectPrev)));
    }

    #[test]
    fn handle_key_k_returns_select_prev() {
        let s = SpecsState::new();
        assert!(matches!(handle_key(&s, &key(KeyCode::Char('k'))), Some(SpecsMessage::SelectPrev)));
    }

    #[test]
    fn handle_key_down_returns_select_next() {
        let s = SpecsState::new();
        assert!(matches!(handle_key(&s, &key(KeyCode::Down)), Some(SpecsMessage::SelectNext)));
    }

    #[test]
    fn handle_key_j_returns_select_next() {
        let s = SpecsState::new();
        assert!(matches!(handle_key(&s, &key(KeyCode::Char('j'))), Some(SpecsMessage::SelectNext)));
    }

    #[test]
    fn handle_key_slash_returns_toggle_search() {
        let s = SpecsState::new();
        assert!(matches!(handle_key(&s, &key(KeyCode::Char('/'))), Some(SpecsMessage::ToggleSearch)));
    }

    #[test]
    fn handle_key_unknown_returns_none() {
        let s = SpecsState::new();
        assert!(handle_key(&s, &key(KeyCode::Char('z'))).is_none());
    }

    // -- handle_key() search mode --

    #[test]
    fn handle_key_search_char() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(matches!(handle_key(&s, &key(KeyCode::Char('a'))), Some(SpecsMessage::SearchChar('a'))));
    }

    #[test]
    fn handle_key_search_backspace() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(matches!(handle_key(&s, &key(KeyCode::Backspace)), Some(SpecsMessage::SearchBackspace)));
    }

    #[test]
    fn handle_key_search_esc_toggles() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(matches!(handle_key(&s, &key(KeyCode::Esc)), Some(SpecsMessage::ToggleSearch)));
    }

    #[test]
    fn handle_key_search_enter_toggles() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(matches!(handle_key(&s, &key(KeyCode::Enter)), Some(SpecsMessage::ToggleSearch)));
    }

    // -- update() --

    #[test]
    fn update_select_prev_saturates_at_zero() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.selected = 0;
        update(&mut s, SpecsMessage::SelectPrev);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn update_select_prev_decrements() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.selected = 2;
        update(&mut s, SpecsMessage::SelectPrev);
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn update_select_next_increments() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.selected = 0;
        update(&mut s, SpecsMessage::SelectNext);
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn update_select_next_stops_at_max() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.selected = 2;
        update(&mut s, SpecsMessage::SelectNext);
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn update_toggle_search_enters_search_mode() {
        let mut s = SpecsState::new();
        assert!(!s.search_mode);
        update(&mut s, SpecsMessage::ToggleSearch);
        assert!(s.search_mode);
    }

    #[test]
    fn update_toggle_search_exits_and_clears_query() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        s.search_query = "test".into();
        s.selected = 1;
        update(&mut s, SpecsMessage::ToggleSearch);
        assert!(!s.search_mode);
        assert!(s.search_query.is_empty());
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn update_search_char_appends_and_resets_selection() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        s.selected = 2;
        update(&mut s, SpecsMessage::SearchChar('a'));
        assert_eq!(s.search_query, "a");
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn update_search_backspace_removes_last_char() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        s.search_query = "abc".into();
        update(&mut s, SpecsMessage::SearchBackspace);
        assert_eq!(s.search_query, "ab");
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn update_search_backspace_on_empty_is_safe() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        update(&mut s, SpecsMessage::SearchBackspace);
        assert!(s.search_query.is_empty());
    }

    // -- render() --

    #[test]
    fn render_smoke_no_specs() {
        let s = SpecsState::new();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
        // No panic
    }

    #[test]
    fn render_smoke_with_specs() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
        // No panic
    }

    #[test]
    fn render_smoke_search_mode() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.search_mode = true;
        s.search_query = "alpha".into();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
        // No panic
    }

    #[test]
    fn render_small_area_returns_early() {
        let s = SpecsState::new();
        let area = Rect::new(0, 0, 80, 2);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
        // No panic, early return for height < 3
    }

    // -- load_specs() --

    #[test]
    fn load_specs_from_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec1 = specs_dir.join("SPEC-100");
        std::fs::create_dir(&spec1).unwrap();
        std::fs::write(
            spec1.join("metadata.json"),
            r#"{"id":"SPEC-100","title":"Test spec","status":"open","phase":"planning"}"#,
        )
        .unwrap();

        let spec2 = specs_dir.join("SPEC-200");
        std::fs::create_dir(&spec2).unwrap();
        std::fs::write(
            spec2.join("metadata.json"),
            r#"{"id":"SPEC-200","title":"Another spec","status":"closed","phase":"done"}"#,
        )
        .unwrap();

        let items = load_specs(dir.path());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "SPEC-100");
        assert_eq!(items[1].id, "SPEC-200");
        assert_eq!(items[0].status, "open");
        assert_eq!(items[1].status, "closed");
    }

    #[test]
    fn load_specs_ignores_non_spec_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        // Non-SPEC directory
        let other = specs_dir.join("not-a-spec");
        std::fs::create_dir(&other).unwrap();

        // SPEC dir without metadata.json
        let no_meta = specs_dir.join("SPEC-999");
        std::fs::create_dir(&no_meta).unwrap();

        let items = load_specs(dir.path());
        assert!(items.is_empty());
    }

    #[test]
    fn load_specs_missing_specs_dir() {
        let dir = tempfile::tempdir().unwrap();
        let items = load_specs(dir.path());
        assert!(items.is_empty());
    }
}
