//! PR Dashboard TUI component (SPEC-1776 Phase 4, T302)
//!
//! Renders a list of PRs with status badges in a ratatui widget.
//!
//! ```text
//! ┌─ Pull Requests ─────────────────────────┐
//! │ #42  feat: add auth   ✓ passing  OPEN  │
//! │ #38  fix: timeout     ✗ failing  OPEN  │
//! │ #35  docs: readme     ✓ passing  MERGED │
//! └─────────────────────────────────────────┘
//! ```

use gwt_core::git::pr_status::{CiStatus, PrState, PrStatus, ReviewStatus};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, StatefulWidget, Widget},
};

/// State for the PR dashboard (selected index, scroll offset, etc.).
#[derive(Debug, Default)]
pub struct PrDashboardState {
    /// Currently selected PR index
    pub selected: Option<usize>,
    /// List of PRs to display
    pub prs: Vec<PrStatus>,
}

impl PrDashboardState {
    pub fn new(prs: Vec<PrStatus>) -> Self {
        let selected = if prs.is_empty() { None } else { Some(0) };
        Self { selected, prs }
    }

    /// Move selection down (wraps around).
    pub fn next(&mut self) {
        if self.prs.is_empty() {
            return;
        }
        let i = self.selected.unwrap_or(0);
        self.selected = Some((i + 1) % self.prs.len());
    }

    /// Move selection up (wraps around).
    pub fn previous(&mut self) {
        if self.prs.is_empty() {
            return;
        }
        let len = self.prs.len();
        let i = self.selected.unwrap_or(0);
        self.selected = Some((i + len - 1) % len);
    }
}

/// The PR dashboard widget.
pub struct PrDashboard;

/// CI status badge character and color.
pub fn ci_status_badge(status: &CiStatus) -> (char, Color) {
    match status {
        CiStatus::Passing => ('\u{2713}', Color::Green),  // ✓
        CiStatus::Failing => ('\u{2717}', Color::Red),    // ✗
        CiStatus::Pending => ('\u{25CB}', Color::Yellow), // ○
        CiStatus::None => ('-', Color::DarkGray),
    }
}

/// PR state color.
pub fn pr_state_color(state: &PrState) -> Color {
    match state {
        PrState::Open => Color::Green,
        PrState::Closed => Color::Red,
        PrState::Merged => Color::Magenta,
    }
}

/// Review status indicator character.
pub fn review_indicator(status: &ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Approved => "\u{2714}",          // ✔
        ReviewStatus::ChangesRequested => "\u{270E}",   // ✎
        ReviewStatus::Pending => "?",
        ReviewStatus::None => "",
    }
}

/// Render a single PR as a Line of styled spans.
pub fn render_pr_line(pr: &PrStatus) -> Line<'static> {
    let (ci_char, ci_color) = ci_status_badge(&pr.ci_status);
    let state_color = pr_state_color(&pr.state);
    let review = review_indicator(&pr.review_status);

    let number_str = format!("#{:<4}", pr.number);
    let title = if pr.title.chars().count() > 30 {
        let truncated: String = pr.title.chars().take(30).collect();
        format!("{truncated}...")
    } else {
        pr.title.clone()
    };
    let ci_label = format!(" {} {} ", ci_char, pr.ci_status);
    let state_label = format!(" {} ", pr.state);

    let mut spans = vec![
        Span::styled(number_str, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::raw(title),
        Span::styled(ci_label, Style::default().fg(ci_color)),
        Span::styled(state_label, Style::default().fg(state_color)),
    ];

    if !review.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            review.to_string(),
            Style::default().fg(Color::Yellow),
        ));
    }

    Line::from(spans)
}

impl StatefulWidget for PrDashboard {
    type State = PrDashboardState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = Block::default()
            .title(" Pull Requests ")
            .borders(Borders::ALL);

        if state.prs.is_empty() {
            let empty_list = List::new(vec![ListItem::new(Line::from(Span::styled(
                "  No pull requests",
                Style::default().fg(Color::DarkGray),
            )))])
            .block(block);
            Widget::render(empty_list, area, buf);
            return;
        }

        let items: Vec<ListItem> = state
            .prs
            .iter()
            .enumerate()
            .map(|(i, pr)| {
                let line = render_pr_line(pr);
                let style = if state.selected == Some(i) {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(items).block(block);
        Widget::render(list, area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pr(number: u64, title: &str, state: PrState, ci: CiStatus) -> PrStatus {
        PrStatus {
            number,
            title: title.to_string(),
            state,
            url: format!("https://example.com/pull/{number}"),
            branch: format!("feature/{number}"),
            ci_status: ci,
            mergeable: true,
            review_status: ReviewStatus::None,
        }
    }

    #[test]
    fn test_ci_status_badge() {
        let (ch, color) = ci_status_badge(&CiStatus::Passing);
        assert_eq!(ch, '\u{2713}');
        assert_eq!(color, Color::Green);

        let (ch, color) = ci_status_badge(&CiStatus::Failing);
        assert_eq!(ch, '\u{2717}');
        assert_eq!(color, Color::Red);

        let (ch, color) = ci_status_badge(&CiStatus::Pending);
        assert_eq!(ch, '\u{25CB}');
        assert_eq!(color, Color::Yellow);

        let (ch, _) = ci_status_badge(&CiStatus::None);
        assert_eq!(ch, '-');
    }

    #[test]
    fn test_pr_state_color() {
        assert_eq!(pr_state_color(&PrState::Open), Color::Green);
        assert_eq!(pr_state_color(&PrState::Closed), Color::Red);
        assert_eq!(pr_state_color(&PrState::Merged), Color::Magenta);
    }

    #[test]
    fn test_review_indicator() {
        assert_eq!(review_indicator(&ReviewStatus::Approved), "\u{2714}");
        assert_eq!(review_indicator(&ReviewStatus::ChangesRequested), "\u{270E}");
        assert_eq!(review_indicator(&ReviewStatus::Pending), "?");
        assert_eq!(review_indicator(&ReviewStatus::None), "");
    }

    #[test]
    fn test_render_empty_pr_list() {
        let mut state = PrDashboardState::default();
        assert!(state.prs.is_empty());
        assert!(state.selected.is_none());

        let area = Rect::new(0, 0, 50, 5);
        let mut buf = Buffer::empty(area);
        StatefulWidget::render(PrDashboard, area, &mut buf, &mut state);

        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("No pull requests"));
    }

    #[test]
    fn test_render_prs_with_status() {
        let prs = vec![
            sample_pr(42, "feat: add auth", PrState::Open, CiStatus::Passing),
            sample_pr(38, "fix: timeout", PrState::Open, CiStatus::Failing),
            sample_pr(35, "docs: readme", PrState::Merged, CiStatus::Passing),
        ];

        let mut state = PrDashboardState::new(prs);
        assert_eq!(state.selected, Some(0));
        assert_eq!(state.prs.len(), 3);

        let area = Rect::new(0, 0, 60, 8);
        let mut buf = Buffer::empty(area);
        StatefulWidget::render(PrDashboard, area, &mut buf, &mut state);

        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(content.contains("#42"));
        assert!(content.contains("#38"));
        assert!(content.contains("#35"));
    }

    #[test]
    fn test_dashboard_state_navigation() {
        let prs = vec![
            sample_pr(1, "PR 1", PrState::Open, CiStatus::Passing),
            sample_pr(2, "PR 2", PrState::Open, CiStatus::Failing),
            sample_pr(3, "PR 3", PrState::Merged, CiStatus::Passing),
        ];

        let mut state = PrDashboardState::new(prs);
        assert_eq!(state.selected, Some(0));

        state.next();
        assert_eq!(state.selected, Some(1));

        state.next();
        assert_eq!(state.selected, Some(2));

        state.next();
        assert_eq!(state.selected, Some(0));

        state.previous();
        assert_eq!(state.selected, Some(2));

        state.previous();
        assert_eq!(state.selected, Some(1));
    }

    #[test]
    fn test_dashboard_state_empty_navigation() {
        let mut state = PrDashboardState::default();
        state.next();
        assert!(state.selected.is_none());
        state.previous();
        assert!(state.selected.is_none());
    }

    #[test]
    fn test_render_pr_line_long_title() {
        let pr = PrStatus {
            number: 100,
            title: "This is a very long title that exceeds thirty characters easily".to_string(),
            state: PrState::Open,
            url: "https://example.com/pull/100".to_string(),
            branch: "feature/long".to_string(),
            ci_status: CiStatus::Pending,
            mergeable: false,
            review_status: ReviewStatus::ChangesRequested,
        };

        let line = render_pr_line(&pr);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("#100"));
        assert!(text.contains("..."));
        assert!(text.contains("\u{270E}")); // ✎ changes requested
    }
}
