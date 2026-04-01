//! SPECs tab — browse and search local SPEC files

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear};

#[derive(Debug, Clone)]
pub struct SpecItem {
    pub dir_name: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub branches: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SpecsState {
    pub specs: Vec<SpecItem>,
    pub selected: usize,
    pub search_query: String,
    pub search_mode: bool,
    pub offset: usize,
    pub detail_mode: bool,
    pub detail_content: String,
    pub detail_scroll: usize,
    pub confirm_launch: bool,
    pub confirm_spec_index: usize,
    pub branch_select_mode: bool,
    pub branch_candidates: Vec<String>,
    pub branch_selected: usize,
}

impl Default for SpecsState {
    fn default() -> Self {
        Self::new()
    }
}

impl SpecsState {
    pub fn new() -> Self {
        Self {
            specs: Vec::new(),
            selected: 0,
            search_query: String::new(),
            search_mode: false,
            offset: 0,
            detail_mode: false,
            detail_content: String::new(),
            detail_scroll: 0,
            confirm_launch: false,
            confirm_spec_index: 0,
            branch_select_mode: false,
            branch_candidates: Vec::new(),
            branch_selected: 0,
        }
    }

    pub fn visible_specs(&self) -> Vec<&SpecItem> {
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
    OpenDetail,
    CloseDetail,
    ScrollDetailUp,
    ScrollDetailDown,
    LaunchAgent,
    ConfirmLaunch,
    CancelLaunch,
    SelectBranch,
    CancelBranchSelect,
    BranchSelectPrev,
    BranchSelectNext,
}

pub fn handle_key(state: &SpecsState, key: &KeyEvent) -> Option<SpecsMessage> {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    // Branch selection mode takes priority
    if state.branch_select_mode {
        return match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(SpecsMessage::BranchSelectNext),
            KeyCode::Up | KeyCode::Char('k') => Some(SpecsMessage::BranchSelectPrev),
            KeyCode::Enter => Some(SpecsMessage::SelectBranch),
            KeyCode::Esc => Some(SpecsMessage::CancelBranchSelect),
            _ => None,
        };
    }

    // Confirm launch dialog
    if state.confirm_launch {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                Some(SpecsMessage::ConfirmLaunch)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                Some(SpecsMessage::CancelLaunch)
            }
            _ => None,
        };
    }

    if state.detail_mode {
        return match key.code {
            KeyCode::Esc => Some(SpecsMessage::CloseDetail),
            KeyCode::Up | KeyCode::Char('k') => Some(SpecsMessage::ScrollDetailUp),
            KeyCode::Down | KeyCode::Char('j') => Some(SpecsMessage::ScrollDetailDown),
            KeyCode::Enter if shift => Some(SpecsMessage::LaunchAgent),
            _ => None,
        };
    }
    if state.search_mode {
        match key.code {
            KeyCode::Esc => Some(SpecsMessage::ToggleSearch),
            KeyCode::Enter if !shift => Some(SpecsMessage::ToggleSearch),
            KeyCode::Enter if shift => None, // Shift+Enter ignored in search
            KeyCode::Char(c) => Some(SpecsMessage::SearchChar(c)),
            KeyCode::Backspace => Some(SpecsMessage::SearchBackspace),
            _ => None,
        }
    } else {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(SpecsMessage::SelectPrev),
            KeyCode::Down | KeyCode::Char('j') => Some(SpecsMessage::SelectNext),
            KeyCode::Char('/') => Some(SpecsMessage::ToggleSearch),
            KeyCode::Enter if shift => Some(SpecsMessage::LaunchAgent),
            KeyCode::Enter => Some(SpecsMessage::OpenDetail),
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
        SpecsMessage::OpenDetail => {
            if !state.visible_specs().is_empty() {
                state.detail_mode = true;
                state.detail_scroll = 0;
                // detail_content is populated by app.rs intercept
            }
        }
        SpecsMessage::CloseDetail => {
            state.detail_mode = false;
            state.detail_content.clear();
            state.detail_scroll = 0;
        }
        SpecsMessage::ScrollDetailUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        SpecsMessage::ScrollDetailDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
        SpecsMessage::LaunchAgent => {
            // Handled by app.rs
        }
        SpecsMessage::ConfirmLaunch => {
            // Handled by app.rs
        }
        SpecsMessage::CancelLaunch => {
            state.confirm_launch = false;
        }
        SpecsMessage::SelectBranch => {
            // Handled by app.rs
        }
        SpecsMessage::CancelBranchSelect => {
            state.branch_select_mode = false;
        }
        SpecsMessage::BranchSelectPrev => {
            state.branch_selected = state.branch_selected.saturating_sub(1);
        }
        SpecsMessage::BranchSelectNext => {
            let max = state.branch_candidates.len(); // includes "+ Create" option
            if state.branch_selected < max {
                state.branch_selected += 1;
            }
        }
    }
}

pub fn render(state: &SpecsState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 {
        return;
    }

    if state.detail_mode {
        render_detail(state, buf, area);
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),    // List
        Constraint::Length(if state.search_mode { 1 } else { 0 }),
    ])
    .split(area);

    // Header
    let count = state.visible_specs().len();
    let header = format!(" SPECs ({count})  [/] Search  [S-Enter] Launch");
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
            "open" => "\u{25CB}",   // ○
            "closed" => "\u{25CF}", // ●
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

    // Overlay dialogs
    if state.confirm_launch {
        render_confirm_dialog(state, buf, area);
    } else if state.branch_select_mode {
        render_branch_select_dialog(state, buf, area);
    }
}

fn render_detail(state: &SpecsState, buf: &mut Buffer, area: Rect) {
    let visible = state.visible_specs();
    let spec_id = visible
        .get(state.selected)
        .map(|s| s.id.as_str())
        .unwrap_or("?");
    let spec_title = visible
        .get(state.selected)
        .map(|s| s.title.as_str())
        .unwrap_or("");

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),    // Content
    ])
    .split(area);

    // Header
    let header = format!(" {spec_id} - {spec_title}  [S-Enter] Launch  [Esc] Back");
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    crate::widgets::markdown::render_markdown(
        buf,
        layout[1],
        &state.detail_content,
        state.detail_scroll,
    );
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn render_confirm_dialog(state: &SpecsState, buf: &mut Buffer, area: Rect) {
    let visible = state.visible_specs();
    let spec = visible.get(state.confirm_spec_index);
    let spec_id = spec.map(|s| s.id.as_str()).unwrap_or("?");
    let phase = spec.map(|s| s.phase.as_str()).unwrap_or("unknown");

    let line1 = format!("SPEC-{spec_id} is in '{phase}' phase.");
    let line2 = "Launch agent anyway? [Y/n]";
    let width = (line1.len().max(line2.len()) + 4) as u16;
    let dialog = centered_rect(width, 6, area);

    Clear.render(dialog, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Confirm Launch ");
    block.render(dialog, buf);

    let inner = dialog.inner(ratatui::layout::Margin::new(1, 1));
    if inner.height >= 2 {
        let s1 = Span::styled(&line1, Style::default().fg(Color::White));
        buf.set_span(inner.x, inner.y, &s1, inner.width);
        let s2 = Span::styled(line2, Style::default().fg(Color::Yellow).bold());
        buf.set_span(inner.x, inner.y + 1, &s2, inner.width);
    }
}

fn render_branch_select_dialog(state: &SpecsState, buf: &mut Buffer, area: Rect) {
    let visible = state.visible_specs();
    let spec = visible.get(state.confirm_spec_index);
    let spec_id = spec.map(|s| s.id.as_str()).unwrap_or("?");

    let create_label = format!("+ Create feature/SPEC-{spec_id}");
    let total = state.branch_candidates.len() + 1; // +1 for create option
    let max_label_len = state
        .branch_candidates
        .iter()
        .map(|b| b.len())
        .chain(std::iter::once(create_label.len()))
        .max()
        .unwrap_or(20);

    let width = (max_label_len + 6) as u16;
    let height = (total + 4) as u16; // borders + title + padding
    let dialog = centered_rect(width, height, area);

    Clear.render(dialog, buf);
    let title = format!(" Select branch for SPEC-{spec_id} ");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title);
    block.render(dialog, buf);

    let inner = dialog.inner(ratatui::layout::Margin::new(1, 1));
    for (i, label) in state
        .branch_candidates
        .iter()
        .map(|b| b.as_str())
        .chain(std::iter::once(create_label.as_str()))
        .enumerate()
    {
        if i as u16 >= inner.height {
            break;
        }
        let is_sel = i == state.branch_selected;
        let style = if is_sel {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };
        let marker = if is_sel { "> " } else { "  " };
        let line = format!("{marker}{label}");
        let span = Span::styled(line, style);
        buf.set_span(inner.x, inner.y + i as u16, &span, inner.width);
    }
}

/// Save a branch name to a SPEC's metadata.json branches array
pub fn save_spec_branch(repo_root: &std::path::Path, spec_dir_name: &str, branch_name: &str) {
    let metadata_path = repo_root
        .join("specs")
        .join(spec_dir_name)
        .join("metadata.json");

    let mut meta: serde_json::Value = match std::fs::read_to_string(&metadata_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(v) => v,
        None => return,
    };

    let branches = meta.as_object_mut().and_then(|obj| {
        if !obj.contains_key("branches") {
            obj.insert("branches".to_string(), serde_json::Value::Array(Vec::new()));
        }
        obj.get_mut("branches").and_then(|v| v.as_array_mut())
    });

    if let Some(arr) = branches {
        let val = serde_json::Value::String(branch_name.to_string());
        if !arr.contains(&val) {
            arr.push(val);
        }
    }

    if let Ok(json_str) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(&metadata_path, json_str);
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
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
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

        let branches = meta["branches"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        items.push(SpecItem {
            dir_name: name.clone(),
            id: meta["id"].as_str().unwrap_or(&name).to_string(),
            title: meta["title"].as_str().unwrap_or("").to_string(),
            status: meta["status"].as_str().unwrap_or("open").to_string(),
            phase: meta["phase"].as_str().unwrap_or("").to_string(),
            branches,
        });
    }

    items.sort_by(|a, b| a.id.cmp(&b.id));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn line_text(buf: &Buffer, area: Rect, row: u16) -> String {
        (area.x..area.right())
            .map(|x| buf[(x, area.y + row)].symbol())
            .collect::<String>()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn sample_specs() -> Vec<SpecItem> {
        vec![
            SpecItem {
                dir_name: "SPEC-1".into(),
                id: "SPEC-1".into(),
                title: "Alpha feature".into(),
                status: "open".into(),
                phase: "planning".into(),
                branches: vec![],
            },
            SpecItem {
                dir_name: "SPEC-2".into(),
                id: "SPEC-2".into(),
                title: "Beta bugfix".into(),
                status: "closed".into(),
                phase: "done".into(),
                branches: vec!["feature/SPEC-2".into()],
            },
            SpecItem {
                dir_name: "SPEC-3".into(),
                id: "SPEC-3".into(),
                title: "Gamma refactor".into(),
                status: "open".into(),
                phase: "implementation".into(),
                branches: vec![],
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
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Up)),
            Some(SpecsMessage::SelectPrev)
        ));
    }

    #[test]
    fn handle_key_k_returns_select_prev() {
        let s = SpecsState::new();
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('k'))),
            Some(SpecsMessage::SelectPrev)
        ));
    }

    #[test]
    fn handle_key_down_returns_select_next() {
        let s = SpecsState::new();
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Down)),
            Some(SpecsMessage::SelectNext)
        ));
    }

    #[test]
    fn handle_key_j_returns_select_next() {
        let s = SpecsState::new();
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('j'))),
            Some(SpecsMessage::SelectNext)
        ));
    }

    #[test]
    fn handle_key_slash_returns_toggle_search() {
        let s = SpecsState::new();
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('/'))),
            Some(SpecsMessage::ToggleSearch)
        ));
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
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('a'))),
            Some(SpecsMessage::SearchChar('a'))
        ));
    }

    #[test]
    fn handle_key_search_backspace() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Backspace)),
            Some(SpecsMessage::SearchBackspace)
        ));
    }

    #[test]
    fn handle_key_search_esc_toggles() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Esc)),
            Some(SpecsMessage::ToggleSearch)
        ));
    }

    #[test]
    fn handle_key_search_enter_toggles() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Enter)),
            Some(SpecsMessage::ToggleSearch)
        ));
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
        assert_eq!(items[0].dir_name, "SPEC-100");
        assert_eq!(items[0].id, "SPEC-100");
        assert_eq!(items[1].id, "SPEC-200");
        assert_eq!(items[0].status, "open");
        assert_eq!(items[1].status, "closed");
    }

    #[test]
    fn load_specs_preserves_directory_name_when_metadata_id_is_numeric() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec = specs_dir.join("SPEC-1776");
        std::fs::create_dir(&spec).unwrap();
        std::fs::write(
            spec.join("metadata.json"),
            r#"{"id":"1776","title":"Numeric id","status":"open","phase":"implementing"}"#,
        )
        .unwrap();

        let items = load_specs(dir.path());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].dir_name, "SPEC-1776");
        assert_eq!(items[0].id, "1776");
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

    // -- detail_mode tests --

    #[test]
    fn detail_open_sets_flag() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        update(&mut s, SpecsMessage::OpenDetail);
        assert!(s.detail_mode);
        assert_eq!(s.detail_scroll, 0);
    }

    #[test]
    fn detail_close_clears_state() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.detail_mode = true;
        s.detail_content = "some content".into();
        s.detail_scroll = 5;
        update(&mut s, SpecsMessage::CloseDetail);
        assert!(!s.detail_mode);
        assert!(s.detail_content.is_empty());
        assert_eq!(s.detail_scroll, 0);
    }

    #[test]
    fn detail_scroll_up_down() {
        let mut s = SpecsState::new();
        s.detail_mode = true;
        update(&mut s, SpecsMessage::ScrollDetailDown);
        assert_eq!(s.detail_scroll, 1);
        update(&mut s, SpecsMessage::ScrollDetailDown);
        assert_eq!(s.detail_scroll, 2);
        update(&mut s, SpecsMessage::ScrollDetailUp);
        assert_eq!(s.detail_scroll, 1);
        update(&mut s, SpecsMessage::ScrollDetailUp);
        assert_eq!(s.detail_scroll, 0);
        // Saturates at zero
        update(&mut s, SpecsMessage::ScrollDetailUp);
        assert_eq!(s.detail_scroll, 0);
    }

    #[test]
    fn handle_key_detail_mode_esc_closes() {
        let mut s = SpecsState::new();
        s.detail_mode = true;
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Esc)),
            Some(SpecsMessage::CloseDetail)
        ));
    }

    #[test]
    fn handle_key_detail_mode_scroll() {
        let mut s = SpecsState::new();
        s.detail_mode = true;
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Up)),
            Some(SpecsMessage::ScrollDetailUp)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Down)),
            Some(SpecsMessage::ScrollDetailDown)
        ));
    }

    #[test]
    fn handle_key_enter_opens_detail() {
        let s = SpecsState::new();
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Enter)),
            Some(SpecsMessage::OpenDetail)
        ));
    }

    #[test]
    fn render_detail_mode_no_panic() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.detail_mode = true;
        s.detail_content = "# SPEC-1\n\nTest content\nLine 2".to_string();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
    }

    #[test]
    fn render_detail_formats_markdown_headings() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.detail_mode = true;
        s.detail_content = "# SPEC-1\n\n- Bullet item".to_string();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);

        render(&s, &mut buf, area);

        let first_content_line = line_text(&buf, area, 1);
        assert!(
            !first_content_line.contains('#'),
            "markdown heading marker should not be rendered literally: {first_content_line:?}"
        );
    }

    // -- T001: branches field tests --

    #[test]
    fn load_specs_reads_branches_from_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec = specs_dir.join("SPEC-500");
        std::fs::create_dir(&spec).unwrap();
        std::fs::write(
            spec.join("metadata.json"),
            r#"{"id":"SPEC-500","title":"With branches","status":"open","phase":"planning","branches":["feature/SPEC-500","fix/hotfix"]}"#,
        )
        .unwrap();

        let items = load_specs(dir.path());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].branches, vec!["feature/SPEC-500", "fix/hotfix"]);
    }

    #[test]
    fn load_specs_empty_branches_when_field_missing() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec = specs_dir.join("SPEC-501");
        std::fs::create_dir(&spec).unwrap();
        std::fs::write(
            spec.join("metadata.json"),
            r#"{"id":"SPEC-501","title":"No branches","status":"open","phase":"planning"}"#,
        )
        .unwrap();

        let items = load_specs(dir.path());
        assert_eq!(items.len(), 1);
        assert!(items[0].branches.is_empty());
    }

    // -- T002: save_spec_branch tests --

    #[test]
    fn save_spec_branch_adds_to_branches() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec = specs_dir.join("SPEC-600");
        std::fs::create_dir(&spec).unwrap();
        std::fs::write(
            spec.join("metadata.json"),
            r#"{"id":"SPEC-600","title":"Test","status":"open","phase":"planning","branches":[]}"#,
        )
        .unwrap();

        save_spec_branch(dir.path(), "SPEC-600", "feature/SPEC-600");

        let items = load_specs(dir.path());
        assert_eq!(items[0].branches, vec!["feature/SPEC-600"]);
    }

    #[test]
    fn save_spec_branch_no_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec = specs_dir.join("SPEC-601");
        std::fs::create_dir(&spec).unwrap();
        std::fs::write(
            spec.join("metadata.json"),
            r#"{"id":"SPEC-601","title":"Test","status":"open","phase":"planning","branches":["feature/SPEC-601"]}"#,
        )
        .unwrap();

        save_spec_branch(dir.path(), "SPEC-601", "feature/SPEC-601");

        let items = load_specs(dir.path());
        assert_eq!(items[0].branches.len(), 1);
    }

    #[test]
    fn save_spec_branch_creates_branches_field_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec = specs_dir.join("SPEC-602");
        std::fs::create_dir(&spec).unwrap();
        std::fs::write(
            spec.join("metadata.json"),
            r#"{"id":"SPEC-602","title":"Test","status":"open","phase":"planning"}"#,
        )
        .unwrap();

        save_spec_branch(dir.path(), "SPEC-602", "feature/new-branch");

        let items = load_specs(dir.path());
        assert_eq!(items[0].branches, vec!["feature/new-branch"]);
    }

    // -- T003: handle_key launch agent tests --

    fn shift_enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
    }

    #[test]
    fn handle_key_shift_enter_returns_launch_agent() {
        let s = SpecsState::new();
        assert!(matches!(
            handle_key(&s, &shift_enter()),
            Some(SpecsMessage::LaunchAgent)
        ));
    }

    #[test]
    fn handle_key_detail_shift_enter_returns_launch_agent() {
        let mut s = SpecsState::new();
        s.detail_mode = true;
        assert!(matches!(
            handle_key(&s, &shift_enter()),
            Some(SpecsMessage::LaunchAgent)
        ));
    }

    #[test]
    fn handle_key_shift_enter_in_search_mode_returns_none() {
        let mut s = SpecsState::new();
        s.search_mode = true;
        assert!(handle_key(&s, &shift_enter()).is_none());
    }

    #[test]
    fn handle_key_normal_enter_still_opens_detail() {
        let s = SpecsState::new();
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Enter)),
            Some(SpecsMessage::OpenDetail)
        ));
    }

    #[test]
    fn handle_key_confirm_y_returns_confirm_launch() {
        let mut s = SpecsState::new();
        s.confirm_launch = true;
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('y'))),
            Some(SpecsMessage::ConfirmLaunch)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Enter)),
            Some(SpecsMessage::ConfirmLaunch)
        ));
    }

    #[test]
    fn handle_key_confirm_esc_returns_cancel_launch() {
        let mut s = SpecsState::new();
        s.confirm_launch = true;
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Esc)),
            Some(SpecsMessage::CancelLaunch)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('n'))),
            Some(SpecsMessage::CancelLaunch)
        ));
    }

    #[test]
    fn handle_key_branch_select_navigation() {
        let mut s = SpecsState::new();
        s.branch_select_mode = true;
        s.branch_candidates = vec!["branch-a".into(), "branch-b".into()];

        assert!(matches!(
            handle_key(&s, &key(KeyCode::Down)),
            Some(SpecsMessage::BranchSelectNext)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('j'))),
            Some(SpecsMessage::BranchSelectNext)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Up)),
            Some(SpecsMessage::BranchSelectPrev)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Char('k'))),
            Some(SpecsMessage::BranchSelectPrev)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Enter)),
            Some(SpecsMessage::SelectBranch)
        ));
        assert!(matches!(
            handle_key(&s, &key(KeyCode::Esc)),
            Some(SpecsMessage::CancelBranchSelect)
        ));
    }

    // -- T005: state field defaults --

    #[test]
    fn new_state_has_launch_fields_defaulted() {
        let s = SpecsState::new();
        assert!(!s.confirm_launch);
        assert_eq!(s.confirm_spec_index, 0);
        assert!(!s.branch_select_mode);
        assert!(s.branch_candidates.is_empty());
        assert_eq!(s.branch_selected, 0);
    }

    // -- T006: overlay render no-panic tests --

    #[test]
    fn render_confirm_launch_no_panic() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.confirm_launch = true;
        s.confirm_spec_index = 0;
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
    }

    #[test]
    fn render_branch_select_no_panic() {
        let mut s = SpecsState::new();
        s.specs = sample_specs();
        s.branch_select_mode = true;
        s.confirm_spec_index = 0;
        s.branch_candidates = vec!["feature/SPEC-1".into(), "fix/hotfix".into()];
        s.branch_selected = 0;
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&s, &mut buf, area);
    }

    // -- update() cancel tests --

    #[test]
    fn update_cancel_launch_clears_flag() {
        let mut s = SpecsState::new();
        s.confirm_launch = true;
        update(&mut s, SpecsMessage::CancelLaunch);
        assert!(!s.confirm_launch);
    }

    #[test]
    fn update_cancel_branch_select_clears_flag() {
        let mut s = SpecsState::new();
        s.branch_select_mode = true;
        update(&mut s, SpecsMessage::CancelBranchSelect);
        assert!(!s.branch_select_mode);
    }

    #[test]
    fn update_branch_select_navigation() {
        let mut s = SpecsState::new();
        s.branch_select_mode = true;
        s.branch_candidates = vec!["a".into(), "b".into()];
        s.branch_selected = 0;

        update(&mut s, SpecsMessage::BranchSelectNext);
        assert_eq!(s.branch_selected, 1);

        update(&mut s, SpecsMessage::BranchSelectPrev);
        assert_eq!(s.branch_selected, 0);

        // Saturates at 0
        update(&mut s, SpecsMessage::BranchSelectPrev);
        assert_eq!(s.branch_selected, 0);
    }
}
