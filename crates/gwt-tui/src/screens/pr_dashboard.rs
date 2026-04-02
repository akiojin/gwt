//! PR Dashboard screen — list and inspect Pull Requests

use crossterm::event::{KeyCode, KeyEvent};
use gwt_git::pr_status::{PrState, PrStatus};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the PR Dashboard screen.
#[derive(Debug, Default)]
pub struct PrDashboardState {
    pub prs: Vec<PrStatus>,
    pub selected: usize,
    pub loading: bool,
    pub detail_mode: bool,
    pub detail_scroll: usize,
}

impl PrDashboardState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_prs(&mut self, prs: Vec<PrStatus>) {
        self.prs = prs;
        self.loading = false;
        self.clamp_selection();
    }

    pub fn clamp_selection(&mut self) {
        if self.prs.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.prs.len() {
            self.selected = self.prs.len() - 1;
        }
    }

    pub fn select_next(&mut self) {
        if self.prs.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.prs.len() - 1);
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn selected_pr(&self) -> Option<&PrStatus> {
        self.prs.get(self.selected)
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages for the PR Dashboard screen.
#[derive(Debug)]
pub enum PrDashboardMessage {
    Refresh,
    SelectNext,
    SelectPrev,
    OpenDetail,
    CloseDetail,
    ScrollDetailUp,
    ScrollDetailDown,
    Loaded(Vec<PrStatus>),
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

pub fn handle_key(state: &PrDashboardState, key: &KeyEvent) -> Option<PrDashboardMessage> {
    if state.detail_mode {
        return match key.code {
            KeyCode::Esc => Some(PrDashboardMessage::CloseDetail),
            KeyCode::Up | KeyCode::Char('k') => Some(PrDashboardMessage::ScrollDetailUp),
            KeyCode::Down | KeyCode::Char('j') => Some(PrDashboardMessage::ScrollDetailDown),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(PrDashboardMessage::SelectNext),
        KeyCode::Char('k') | KeyCode::Up => Some(PrDashboardMessage::SelectPrev),
        KeyCode::Char('r') => Some(PrDashboardMessage::Refresh),
        KeyCode::Enter => Some(PrDashboardMessage::OpenDetail),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

pub fn update(state: &mut PrDashboardState, msg: PrDashboardMessage) {
    match msg {
        PrDashboardMessage::SelectNext => state.select_next(),
        PrDashboardMessage::SelectPrev => state.select_prev(),
        PrDashboardMessage::Refresh => {
            state.loading = true;
        }
        PrDashboardMessage::Loaded(prs) => {
            state.set_prs(prs);
        }
        PrDashboardMessage::OpenDetail => {
            if state.selected_pr().is_some() {
                state.detail_mode = true;
                state.detail_scroll = 0;
            }
        }
        PrDashboardMessage::CloseDetail => {
            state.detail_mode = false;
            state.detail_scroll = 0;
        }
        PrDashboardMessage::ScrollDetailUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        PrDashboardMessage::ScrollDetailDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(state: &PrDashboardState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    if state.detail_mode {
        render_detail(state, buf, area);
        return;
    }

    let header_height = 2u16;
    let list_height = area.height.saturating_sub(header_height);

    let header_area = Rect::new(area.x, area.y, area.width, header_height);
    let list_area = Rect::new(area.x, area.y + header_height, area.width, list_height);

    render_header(state, buf, header_area);
    render_list(state, buf, list_area);
}

fn render_header(state: &PrDashboardState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    let title = if state.loading {
        " Pull Requests (loading...)".to_string()
    } else {
        format!(" Pull Requests ({})", state.prs.len())
    };

    let title_span = Span::styled(title, Style::default().fg(Color::White).bold());
    buf.set_line(area.x, area.y, &Line::from(vec![title_span]), area.width);

    if area.height >= 2 {
        let hints = Line::from(vec![
            Span::styled(" [Enter] Detail", Style::default().fg(Color::DarkGray)),
            Span::styled("  ", Style::default()),
            Span::styled("[r] Refresh", Style::default().fg(Color::DarkGray)),
        ]);
        buf.set_line(area.x, area.y + 1, &hints, area.width);
    }
}

fn render_list(state: &PrDashboardState, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }

    if state.prs.is_empty() {
        let msg = if state.loading {
            "Loading..."
        } else {
            "No pull requests found"
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
    let offset = if state.selected >= viewport {
        state.selected - viewport + 1
    } else {
        0
    };

    for (row, pr) in state.prs.iter().skip(offset).take(viewport).enumerate() {
        let is_selected = row + offset == state.selected;
        let y = area.y + row as u16;
        render_pr_row(pr, is_selected, buf, area.x, y, area.width);
    }
}

fn pr_state_color(state: PrState) -> Color {
    match state {
        PrState::Open => Color::Green,
        PrState::Closed => Color::Red,
        PrState::Merged => Color::Magenta,
    }
}

fn ci_status_color(status: &str) -> Color {
    match status {
        "SUCCESS" => Color::Green,
        "FAILURE" => Color::Red,
        "PENDING" => Color::Yellow,
        _ => Color::DarkGray,
    }
}

fn render_pr_row(pr: &PrStatus, is_selected: bool, buf: &mut Buffer, x: u16, y: u16, width: u16) {
    let sel = if is_selected { ">" } else { " " };
    let sel_style = if is_selected {
        Style::default().fg(Color::White).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let state_str = pr.state.to_string();
    let state_color = pr_state_color(pr.state);

    let ci_icon = match pr.ci_status.as_str() {
        "SUCCESS" => "\u{2714}",  // checkmark
        "FAILURE" => "\u{2718}",  // cross
        "PENDING" => "\u{25CB}",  // circle
        _ => "\u{2015}",         // dash
    };
    let ci_color = ci_status_color(&pr.ci_status);

    let mut spans = vec![
        Span::styled(sel, sel_style),
        Span::styled(format!(" #{}", pr.number), Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {state_str}"), Style::default().fg(state_color)),
        Span::styled(format!(" {ci_icon}"), Style::default().fg(ci_color)),
    ];

    // Title (fill remaining)
    let used: usize = spans.iter().map(|s| s.content.len()).sum();
    let remaining = (width as usize).saturating_sub(used + 1);
    if remaining > 3 {
        let display_title = if pr.title.chars().count() > remaining {
            let truncated: String = pr.title.chars().take(remaining - 4).collect();
            format!(" {truncated}...")
        } else {
            format!(" {}", pr.title)
        };
        spans.push(Span::styled(
            display_title,
            Style::default().fg(Color::White),
        ));
    }

    if is_selected {
        for col in x..x + width {
            buf[(col, y)].set_style(Style::default().bg(Color::Rgb(40, 40, 60)));
        }
    }

    buf.set_line(x, y, &Line::from(spans), width);
}

fn render_detail(state: &PrDashboardState, buf: &mut Buffer, area: Rect) {
    let Some(pr) = state.selected_pr() else {
        return;
    };

    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(1),   // Content
    ])
    .split(area);

    // Header
    let header = format!(" #{} {}  [Esc] Back", pr.number, pr.title);
    let header_span = Span::styled(header, Style::default().fg(Color::Cyan).bold());
    buf.set_span(layout[0].x, layout[0].y, &header_span, layout[0].width);

    // Detail content
    let content_area = layout[1];
    if content_area.height == 0 {
        return;
    }

    let lines = build_detail_lines(pr);
    let scroll = state.detail_scroll;
    for (i, line) in lines.iter().skip(scroll).enumerate() {
        let y = content_area.y + i as u16;
        if y >= content_area.y + content_area.height {
            break;
        }
        buf.set_line(content_area.x, y, line, content_area.width);
    }
}

fn build_detail_lines(pr: &PrStatus) -> Vec<Line<'static>> {
    let state_color = pr_state_color(pr.state);
    let ci_color = ci_status_color(&pr.ci_status);

    let merge_color = match pr.mergeable.as_str() {
        "MERGEABLE" => Color::Green,
        "CONFLICTING" => Color::Red,
        _ => Color::DarkGray,
    };

    let review_color = match pr.review_status.as_str() {
        "APPROVED" => Color::Green,
        "CHANGES_REQUESTED" => Color::Red,
        "REVIEW_REQUIRED" => Color::Yellow,
        _ => Color::DarkGray,
    };

    vec![
        Line::from(vec![
            Span::styled("  State:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(pr.state.to_string(), Style::default().fg(state_color)),
        ]),
        Line::from(vec![
            Span::styled("  CI:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(pr.ci_status.clone(), Style::default().fg(ci_color)),
        ]),
        Line::from(vec![
            Span::styled("  Merge:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(pr.mergeable.clone(), Style::default().fg(merge_color)),
        ]),
        Line::from(vec![
            Span::styled("  Review:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(pr.review_status.clone(), Style::default().fg(review_color)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  URL: ", Style::default().fg(Color::DarkGray)),
            Span::styled(pr.url.clone(), Style::default().fg(Color::Blue)),
        ]),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_pr(number: u64, title: &str, state: PrState) -> PrStatus {
        PrStatus {
            number,
            title: title.to_string(),
            state,
            url: format!("https://github.com/owner/repo/pull/{number}"),
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            review_status: "APPROVED".to_string(),
        }
    }

    // -- State tests --

    #[test]
    fn set_prs_resets() {
        let mut state = PrDashboardState::new();
        state.loading = true;
        state.selected = 99;
        state.set_prs(vec![
            make_pr(1, "PR A", PrState::Open),
            make_pr(2, "PR B", PrState::Merged),
        ]);
        assert!(!state.loading);
        assert_eq!(state.prs.len(), 2);
        assert_eq!(state.selected, 1); // clamped
    }

    #[test]
    fn select_next_prev() {
        let mut state = PrDashboardState::new();
        state.set_prs(vec![
            make_pr(1, "A", PrState::Open),
            make_pr(2, "B", PrState::Open),
            make_pr(3, "C", PrState::Open),
        ]);
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
    fn selected_pr_returns_correct() {
        let mut state = PrDashboardState::new();
        state.set_prs(vec![
            make_pr(1, "A", PrState::Open),
            make_pr(2, "B", PrState::Merged),
        ]);
        state.selected = 1;
        assert_eq!(state.selected_pr().unwrap().number, 2);
    }

    #[test]
    fn empty_state_selected_pr_none() {
        let state = PrDashboardState::new();
        assert!(state.selected_pr().is_none());
    }

    // -- Key handling tests --

    #[test]
    fn handle_key_navigation() {
        let state = PrDashboardState::new();
        let key_j = KeyEvent::new(KeyCode::Char('j'), crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_j),
            Some(PrDashboardMessage::SelectNext)
        ));

        let key_k = KeyEvent::new(KeyCode::Char('k'), crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_k),
            Some(PrDashboardMessage::SelectPrev)
        ));
    }

    #[test]
    fn handle_key_detail() {
        let state = PrDashboardState::new();
        let key_enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_enter),
            Some(PrDashboardMessage::OpenDetail)
        ));
    }

    #[test]
    fn handle_key_refresh() {
        let state = PrDashboardState::new();
        let key_r = KeyEvent::new(KeyCode::Char('r'), crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_r),
            Some(PrDashboardMessage::Refresh)
        ));
    }

    #[test]
    fn handle_key_detail_mode() {
        let mut state = PrDashboardState::new();
        state.detail_mode = true;

        let key_esc = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_esc),
            Some(PrDashboardMessage::CloseDetail)
        ));

        let key_up = KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_up),
            Some(PrDashboardMessage::ScrollDetailUp)
        ));
    }

    // -- Update tests --

    #[test]
    fn update_loaded() {
        let mut state = PrDashboardState::new();
        state.loading = true;
        update(
            &mut state,
            PrDashboardMessage::Loaded(vec![make_pr(1, "PR", PrState::Open)]),
        );
        assert!(!state.loading);
        assert_eq!(state.prs.len(), 1);
    }

    #[test]
    fn update_detail_open_close() {
        let mut state = PrDashboardState::new();
        state.set_prs(vec![make_pr(1, "PR", PrState::Open)]);
        update(&mut state, PrDashboardMessage::OpenDetail);
        assert!(state.detail_mode);
        update(&mut state, PrDashboardMessage::CloseDetail);
        assert!(!state.detail_mode);
    }

    #[test]
    fn update_detail_scroll() {
        let mut state = PrDashboardState::new();
        state.detail_mode = true;
        update(&mut state, PrDashboardMessage::ScrollDetailDown);
        assert_eq!(state.detail_scroll, 1);
        update(&mut state, PrDashboardMessage::ScrollDetailUp);
        assert_eq!(state.detail_scroll, 0);
    }

    // -- Render tests --

    #[test]
    fn render_empty_state() {
        let state = PrDashboardState::new();
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
    fn render_with_prs() {
        let mut state = PrDashboardState::new();
        state.set_prs(vec![
            make_pr(1, "Add feature", PrState::Open),
            make_pr(2, "Fix bug", PrState::Merged),
            make_pr(3, "WIP", PrState::Closed),
        ]);

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
    fn render_small_area_does_not_panic() {
        let state = PrDashboardState::new();
        let backend = TestBackend::new(5, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_detail_mode() {
        let mut state = PrDashboardState::new();
        state.set_prs(vec![make_pr(1, "Test PR", PrState::Open)]);
        state.detail_mode = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    // -- Detail line building --

    #[test]
    fn build_detail_lines_has_fields() {
        let pr = make_pr(1, "Test", PrState::Open);
        let lines = build_detail_lines(&pr);
        assert!(lines.len() >= 5);
    }
}
