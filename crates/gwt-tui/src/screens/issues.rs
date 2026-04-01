//! Issues/SPECs screen — list GitHub Issues and local SPECs with search

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

// ---------------------------------------------------------------------------
// Issue item
// ---------------------------------------------------------------------------

/// A single issue entry for display.
#[derive(Debug, Clone)]
pub struct IssueItem {
    pub number: u64,
    pub title: String,
    pub is_spec: bool,
    pub spec_id: Option<String>,
    pub state: String,
    pub labels: Vec<String>,
}

impl IssueItem {
    /// Matches search query against issue number, title, labels, or spec_id.
    pub fn matches_search(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        if self.number.to_string().contains(&q) {
            return true;
        }
        if self.title.to_lowercase().contains(&q) {
            return true;
        }
        if let Some(ref sid) = self.spec_id {
            if sid.to_lowercase().contains(&q) {
                return true;
            }
        }
        self.labels.iter().any(|l| l.to_lowercase().contains(&q))
    }

    /// State color for display.
    pub fn state_color(&self) -> Color {
        match self.state.as_str() {
            "open" | "OPEN" => Color::Green,
            "closed" | "CLOSED" => Color::Red,
            _ => Color::DarkGray,
        }
    }
}

// ---------------------------------------------------------------------------
// Issue panel state
// ---------------------------------------------------------------------------

/// State for the issues screen.
#[derive(Debug, Default)]
pub struct IssuePanelState {
    pub issues: Vec<IssueItem>,
    pub selected: usize,
    pub search_query: String,
    pub search_mode: bool,
    pub loading: bool,
    pub detail_mode: bool,
    pub detail_content: String,
    pub detail_scroll: usize,
}

impl IssuePanelState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return indices of issues matching the current search.
    pub fn filtered_indices(&self) -> Vec<usize> {
        self.issues
            .iter()
            .enumerate()
            .filter(|(_, issue)| issue.matches_search(&self.search_query))
            .map(|(i, _)| i)
            .collect()
    }

    /// Count of visible issues.
    pub fn visible_count(&self) -> usize {
        self.filtered_indices().len()
    }

    /// Clamp selected to visible range.
    pub fn clamp_selection(&mut self) {
        let count = self.visible_count();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        let count = self.visible_count();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + 1).min(count - 1);
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Get the currently selected IssueItem, if any.
    pub fn selected_issue(&self) -> Option<&IssueItem> {
        let indices = self.filtered_indices();
        indices.get(self.selected).and_then(|&i| self.issues.get(i))
    }

    /// Set issues and reset selection.
    pub fn set_issues(&mut self, issues: Vec<IssueItem>) {
        self.issues = issues;
        self.clamp_selection();
        self.loading = false;
    }

    /// Toggle search input mode.
    pub fn toggle_search(&mut self) {
        self.search_mode = !self.search_mode;
    }

    /// Clear search text and exit search mode.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.search_mode = false;
        self.clamp_selection();
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages for the issues screen.
#[derive(Debug)]
pub enum IssuesMessage {
    Refresh,
    SelectNext,
    SelectPrev,
    ToggleSearch,
    SearchInput(char),
    SearchBackspace,
    SearchClear,
    Enter,
    Loaded(Vec<IssueItem>),
    OpenDetail,
    CloseDetail,
    ScrollDetailUp,
    ScrollDetailDown,
    /// Launch an agent for the selected issue
    LaunchAgent,
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Handle a key event for the issues screen.
pub fn handle_key(state: &IssuePanelState, key: &KeyEvent) -> Option<IssuesMessage> {
    if state.detail_mode {
        return match key.code {
            KeyCode::Esc => Some(IssuesMessage::CloseDetail),
            KeyCode::Up | KeyCode::Char('k') => Some(IssuesMessage::ScrollDetailUp),
            KeyCode::Down | KeyCode::Char('j') => Some(IssuesMessage::ScrollDetailDown),
            _ => None,
        };
    }
    if state.search_mode {
        return handle_search_key(key);
    }

    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(IssuesMessage::SelectNext),
        KeyCode::Char('k') | KeyCode::Up => Some(IssuesMessage::SelectPrev),
        KeyCode::Char('/') => Some(IssuesMessage::ToggleSearch),
        KeyCode::Char('r') => Some(IssuesMessage::Refresh),
        KeyCode::Enter if shift => Some(IssuesMessage::LaunchAgent),
        KeyCode::Enter => Some(IssuesMessage::OpenDetail),
        _ => None,
    }
}

/// Handle key events in search input mode.
fn handle_search_key(key: &KeyEvent) -> Option<IssuesMessage> {
    match key.code {
        KeyCode::Esc => Some(IssuesMessage::ToggleSearch),
        KeyCode::Enter => Some(IssuesMessage::ToggleSearch),
        KeyCode::Backspace => Some(IssuesMessage::SearchBackspace),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(IssuesMessage::SearchClear)
        }
        KeyCode::Char(c) => Some(IssuesMessage::SearchInput(c)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Apply an IssuesMessage to the IssuePanelState.
pub fn update(state: &mut IssuePanelState, msg: IssuesMessage) {
    match msg {
        IssuesMessage::SelectNext => state.select_next(),
        IssuesMessage::SelectPrev => state.select_prev(),
        IssuesMessage::ToggleSearch => state.toggle_search(),
        IssuesMessage::SearchInput(c) => {
            state.search_query.push(c);
            state.clamp_selection();
        }
        IssuesMessage::SearchBackspace => {
            state.search_query.pop();
            state.clamp_selection();
        }
        IssuesMessage::SearchClear => state.clear_search(),
        IssuesMessage::Refresh => {
            state.loading = true;
        }
        IssuesMessage::Loaded(issues) => {
            state.set_issues(issues);
        }
        IssuesMessage::Enter => {
            // Handled at app level (intercepted for OpenDetail).
        }
        IssuesMessage::OpenDetail => {
            if state.selected_issue().is_some() {
                state.detail_mode = true;
                state.detail_scroll = 0;
                // detail_content is populated by app.rs intercept
            }
        }
        IssuesMessage::CloseDetail => {
            state.detail_mode = false;
            state.detail_content.clear();
            state.detail_scroll = 0;
        }
        IssuesMessage::ScrollDetailUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        IssuesMessage::ScrollDetailDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
        IssuesMessage::LaunchAgent => {
            // Handled by app.rs
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the issues screen.
pub fn render(state: &IssuePanelState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    if state.detail_mode {
        render_detail(state, buf, area);
        return;
    }

    let footer_height = if state.search_mode { 1 } else { 0 };
    let header_height = 2u16;
    let list_height = area.height.saturating_sub(header_height + footer_height);

    let header_area = Rect::new(area.x, area.y, area.width, header_height);
    let list_area = Rect::new(area.x, area.y + header_height, area.width, list_height);
    let footer_area = Rect::new(
        area.x,
        area.y + header_height + list_height,
        area.width,
        footer_height,
    );

    render_header(state, buf, header_area);
    render_list(state, buf, list_area);
    if state.search_mode {
        render_search_bar(state, buf, footer_area);
    }
}

/// Render header with title and key hints.
fn render_header(state: &IssuePanelState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    let visible = state.visible_count();
    let total = state.issues.len();
    let title = if state.loading {
        " Issues (loading...)".to_string()
    } else if visible == total {
        format!(" Issues ({total})")
    } else {
        format!(" Issues ({visible}/{total})")
    };

    let title_span = Span::styled(title, Style::default().fg(Color::White).bold());
    buf.set_line(area.x, area.y, &Line::from(vec![title_span]), area.width);

    if area.height >= 2 {
        let hints = Line::from(vec![
            Span::styled(" [/] Search", Style::default().fg(Color::DarkGray)),
            Span::styled("  ", Style::default()),
            Span::styled("[r] Refresh", Style::default().fg(Color::DarkGray)),
        ]);
        buf.set_line(area.x, area.y + 1, &hints, area.width);
    }
}

/// Render issue list rows.
fn render_list(state: &IssuePanelState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    let indices = state.filtered_indices();

    if indices.is_empty() {
        let msg = if state.search_query.is_empty() {
            "No issues found"
        } else {
            "No matching issues"
        };
        let para = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        let y = area.y + area.height / 2;
        let text_area = Rect::new(area.x, y, area.width, 1);
        ratatui::widgets::Widget::render(para, text_area, buf);
        return;
    }

    let viewport = area.height as usize;

    // Simple scroll: keep selected visible.
    let offset = if state.selected >= viewport {
        state.selected - viewport + 1
    } else {
        0
    };

    for (row, vis_idx) in indices.iter().skip(offset).take(viewport).enumerate() {
        let issue = &state.issues[*vis_idx];
        let is_selected = row + offset == state.selected;
        let y = area.y + row as u16;

        render_issue_row(issue, is_selected, buf, area.x, y, area.width);
    }
}

/// Render a single issue row.
fn render_issue_row(
    issue: &IssueItem,
    is_selected: bool,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
) {
    let mut spans: Vec<Span> = Vec::new();

    // Selection indicator
    let sel = if is_selected { ">" } else { " " };
    let sel_style = if is_selected {
        Style::default().fg(Color::White).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };
    spans.push(Span::styled(sel, sel_style));

    // SPEC badge
    if issue.is_spec {
        spans.push(Span::styled(
            " SPEC",
            Style::default().fg(Color::Cyan).bold(),
        ));
        if let Some(ref sid) = issue.spec_id {
            spans.push(Span::styled(
                format!(" {sid}"),
                Style::default().fg(Color::Cyan),
            ));
        }
    }

    // Issue number
    spans.push(Span::styled(
        format!(" #{}", issue.number),
        Style::default().fg(Color::DarkGray),
    ));

    // State
    spans.push(Span::styled(
        format!(" {}", issue.state),
        Style::default().fg(issue.state_color()),
    ));

    // Title (fill remaining)
    let used: usize = spans.iter().map(|s| s.content.len()).sum();
    let remaining = (width as usize).saturating_sub(used + 1);
    if remaining > 3 {
        let display_title = if issue.title.len() > remaining {
            format!(" {}...", &issue.title[..remaining - 4])
        } else {
            format!(" {}", issue.title)
        };
        spans.push(Span::styled(
            display_title,
            Style::default().fg(Color::White),
        ));
    }

    // Background highlight
    if is_selected {
        for col in x..x + width {
            buf[(col, y)].set_style(Style::default().bg(Color::Rgb(40, 40, 60)));
        }
    }

    let line = Line::from(spans);
    buf.set_line(x, y, &line, width);
}

/// Render the search input bar.
fn render_search_bar(state: &IssuePanelState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }
    let line = Line::from(vec![
        Span::styled(" /", Style::default().fg(Color::Cyan).bold()),
        Span::styled(&state.search_query, Style::default().fg(Color::White)),
        Span::styled(
            "_",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);
    buf.set_line(area.x, area.y, &line, area.width);
}

/// Render detail view for a selected issue.
fn render_detail(state: &IssuePanelState, buf: &mut Buffer, area: Rect) {
    let issue = state.selected_issue();
    let title = issue.map(|i| i.title.as_str()).unwrap_or("?");
    let number = issue.map(|i| i.number).unwrap_or(0);

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),    // Content
    ])
    .split(area);

    // Header
    let header = format!(" #{number} {title}  [Esc] Back");
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    crate::widgets::markdown::render_markdown(
        buf,
        layout[1],
        &state.detail_content,
        state.detail_scroll,
    );
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

/// Scan `specs/SPEC-*/metadata.json` to populate the issue list with local SPECs.
pub fn load_specs(repo_root: &std::path::Path) -> Vec<IssueItem> {
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
        let title = std::fs::read_to_string(&metadata_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["title"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| name.clone());

        let status = std::fs::read_to_string(&metadata_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["status"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "open".to_string());

        let number = name.trim_start_matches("SPEC-").parse::<u64>().unwrap_or(0);

        items.push(IssueItem {
            number,
            title,
            is_spec: true,
            spec_id: Some(name),
            state: status,
            labels: vec!["spec".to_string()],
        });
    }

    items.sort_by(|a, b| b.number.cmp(&a.number));
    items
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_issue(number: u64, title: &str, state: &str) -> IssueItem {
        IssueItem {
            number,
            title: title.to_string(),
            is_spec: false,
            spec_id: None,
            state: state.to_string(),
            labels: Vec::new(),
        }
    }

    fn make_spec_issue(number: u64, title: &str, spec_id: &str) -> IssueItem {
        IssueItem {
            number,
            title: title.to_string(),
            is_spec: true,
            spec_id: Some(spec_id.to_string()),
            state: "open".to_string(),
            labels: vec!["gwt-spec".to_string()],
        }
    }

    // -- IssueItem tests --

    #[test]
    fn issue_matches_search_by_title() {
        let issue = make_issue(1, "Fix login bug", "open");
        assert!(issue.matches_search("login"));
        assert!(issue.matches_search("FIX"));
        assert!(!issue.matches_search("payment"));
    }

    #[test]
    fn issue_matches_search_by_number() {
        let issue = make_issue(42, "Some issue", "open");
        assert!(issue.matches_search("42"));
        assert!(!issue.matches_search("99"));
    }

    #[test]
    fn issue_matches_search_by_spec_id() {
        let issue = make_spec_issue(10, "SPEC title", "SPEC-1776");
        assert!(issue.matches_search("1776"));
        assert!(issue.matches_search("SPEC"));
    }

    #[test]
    fn issue_matches_search_by_label() {
        let mut issue = make_issue(1, "Title", "open");
        issue.labels = vec!["bug".to_string(), "critical".to_string()];
        assert!(issue.matches_search("bug"));
        assert!(issue.matches_search("critical"));
        assert!(!issue.matches_search("feature"));
    }

    #[test]
    fn issue_empty_search_matches_all() {
        let issue = make_issue(1, "Any", "open");
        assert!(issue.matches_search(""));
    }

    #[test]
    fn issue_state_color() {
        assert_eq!(make_issue(1, "t", "open").state_color(), Color::Green);
        assert_eq!(make_issue(1, "t", "OPEN").state_color(), Color::Green);
        assert_eq!(make_issue(1, "t", "closed").state_color(), Color::Red);
    }

    // -- IssuePanelState tests --

    #[test]
    fn state_filtered_indices_with_search() {
        let mut state = IssuePanelState::new();
        state.issues = vec![
            make_issue(1, "Fix auth", "open"),
            make_issue(2, "Add payments", "open"),
            make_issue(3, "Auth refactor", "closed"),
        ];

        state.search_query = "auth".to_string();
        let indices = state.filtered_indices();
        assert_eq!(indices.len(), 2);
        assert_eq!(state.issues[indices[0]].number, 1);
        assert_eq!(state.issues[indices[1]].number, 3);
    }

    #[test]
    fn state_select_next_prev() {
        let mut state = IssuePanelState::new();
        state.issues = vec![
            make_issue(1, "A", "open"),
            make_issue(2, "B", "open"),
            make_issue(3, "C", "open"),
        ];
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_next();
        assert_eq!(state.selected, 2);

        state.select_next();
        assert_eq!(state.selected, 2); // clamped

        state.select_prev();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn state_clamp_selection() {
        let mut state = IssuePanelState::new();
        state.selected = 10;
        state.issues = vec![make_issue(1, "A", "open")];
        state.clamp_selection();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn state_toggle_search() {
        let mut state = IssuePanelState::new();
        assert!(!state.search_mode);
        state.toggle_search();
        assert!(state.search_mode);
        state.toggle_search();
        assert!(!state.search_mode);
    }

    #[test]
    fn state_clear_search() {
        let mut state = IssuePanelState::new();
        state.search_query = "test".to_string();
        state.search_mode = true;
        state.clear_search();
        assert!(state.search_query.is_empty());
        assert!(!state.search_mode);
    }

    #[test]
    fn state_set_issues() {
        let mut state = IssuePanelState::new();
        state.loading = true;
        state.selected = 99;

        state.set_issues(vec![make_issue(1, "A", "open"), make_issue(2, "B", "open")]);
        assert!(!state.loading);
        assert_eq!(state.issues.len(), 2);
        assert_eq!(state.selected, 1); // clamped
    }

    // -- Key handling tests --

    #[test]
    fn handle_key_navigation() {
        let state = IssuePanelState::new();

        let key_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_j),
            Some(IssuesMessage::SelectNext)
        ));

        let key_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_k),
            Some(IssuesMessage::SelectPrev)
        ));
    }

    #[test]
    fn handle_key_search_mode() {
        let mut state = IssuePanelState::new();
        state.search_mode = true;

        let key_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_a),
            Some(IssuesMessage::SearchInput('a'))
        ));

        let key_esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_esc),
            Some(IssuesMessage::ToggleSearch)
        ));
    }

    // -- Update tests --

    #[test]
    fn update_search_input() {
        let mut state = IssuePanelState::new();
        state.issues = vec![make_issue(1, "A", "open")];
        state.search_mode = true;

        update(&mut state, IssuesMessage::SearchInput('t'));
        update(&mut state, IssuesMessage::SearchInput('e'));
        assert_eq!(state.search_query, "te");

        update(&mut state, IssuesMessage::SearchBackspace);
        assert_eq!(state.search_query, "t");

        update(&mut state, IssuesMessage::SearchClear);
        assert!(state.search_query.is_empty());
        assert!(!state.search_mode);
    }

    #[test]
    fn update_loaded() {
        let mut state = IssuePanelState::new();
        state.loading = true;

        let issues = vec![make_issue(1, "A", "open")];
        update(&mut state, IssuesMessage::Loaded(issues));
        assert!(!state.loading);
        assert_eq!(state.issues.len(), 1);
    }

    // -- Render tests --

    #[test]
    fn render_empty_state() {
        let state = IssuePanelState::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_with_issues() {
        let mut state = IssuePanelState::new();
        state.issues = vec![
            make_issue(1, "Fix auth bug", "open"),
            make_spec_issue(2, "SPEC: New feature", "SPEC-1776"),
            make_issue(3, "Closed issue", "closed"),
        ];

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_with_search_mode() {
        let mut state = IssuePanelState::new();
        state.issues = vec![make_issue(1, "Title", "open")];
        state.search_mode = true;
        state.search_query = "test".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_small_area_does_not_panic() {
        let state = IssuePanelState::new();
        let backend = TestBackend::new(5, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    // -- Detail mode tests --

    #[test]
    fn detail_open_close() {
        let mut state = IssuePanelState::new();
        state.issues = vec![make_issue(1, "Test", "open")];
        update(&mut state, IssuesMessage::OpenDetail);
        assert!(state.detail_mode);
        assert_eq!(state.detail_scroll, 0);
        state.detail_content = "detail text".to_string();
        update(&mut state, IssuesMessage::CloseDetail);
        assert!(!state.detail_mode);
        assert!(state.detail_content.is_empty());
    }

    #[test]
    fn detail_scroll() {
        let mut state = IssuePanelState::new();
        state.detail_mode = true;
        update(&mut state, IssuesMessage::ScrollDetailDown);
        assert_eq!(state.detail_scroll, 1);
        update(&mut state, IssuesMessage::ScrollDetailUp);
        assert_eq!(state.detail_scroll, 0);
        update(&mut state, IssuesMessage::ScrollDetailUp);
        assert_eq!(state.detail_scroll, 0);
    }

    #[test]
    fn handle_key_detail_mode() {
        let mut state = IssuePanelState::new();
        state.detail_mode = true;
        let key_esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_esc),
            Some(IssuesMessage::CloseDetail)
        ));
        let key_up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_up),
            Some(IssuesMessage::ScrollDetailUp)
        ));
        let key_down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_down),
            Some(IssuesMessage::ScrollDetailDown)
        ));
    }

    #[test]
    fn render_detail_mode() {
        let mut state = IssuePanelState::new();
        state.issues = vec![make_issue(1, "Bug Fix", "open")];
        state.detail_mode = true;
        state.detail_content = "Detail line 1\nDetail line 2".to_string();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }
}
