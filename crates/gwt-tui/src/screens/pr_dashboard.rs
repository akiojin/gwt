//! PR Dashboard screen.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// PR state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Closed => "Closed",
            Self::Merged => "Merged",
        }
    }

    /// Color for display.
    pub fn color(self) -> Color {
        match self {
            Self::Open => Color::Green,
            Self::Closed => Color::Red,
            Self::Merged => Color::Magenta,
        }
    }
}

/// A single PR entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrItem {
    pub number: u32,
    pub title: String,
    pub state: PrState,
    pub ci_status: String,
    pub mergeable: bool,
    pub review_status: String,
}

/// State for the PR dashboard screen.
#[derive(Debug, Clone, Default)]
pub struct PrDashboardState {
    pub(crate) prs: Vec<PrItem>,
    pub(crate) selected: usize,
    pub(crate) detail_view: bool,
}

impl PrDashboardState {
    /// Get the currently selected PR, if any.
    pub fn selected_pr(&self) -> Option<&PrItem> {
        self.prs.get(self.selected)
    }

    /// Clamp selected index to list length.
    fn clamp_selected(&mut self) {
        super::clamp_index(&mut self.selected, self.prs.len());
    }
}

/// Messages specific to the PR dashboard screen.
#[derive(Debug, Clone)]
pub enum PrDashboardMessage {
    MoveUp,
    MoveDown,
    ToggleDetail,
    Refresh,
    SetPrs(Vec<PrItem>),
}

/// Update PR dashboard state in response to a message.
pub fn update(state: &mut PrDashboardState, msg: PrDashboardMessage) {
    match msg {
        PrDashboardMessage::MoveUp => {
            super::move_up(&mut state.selected, state.prs.len());
        }
        PrDashboardMessage::MoveDown => {
            super::move_down(&mut state.selected, state.prs.len());
        }
        PrDashboardMessage::ToggleDetail => {
            if !state.prs.is_empty() {
                state.detail_view = !state.detail_view;
            }
        }
        PrDashboardMessage::Refresh => {
            // Signal to reload -- handled by caller
        }
        PrDashboardMessage::SetPrs(prs) => {
            state.prs = prs;
            state.clamp_selected();
        }
    }
}

/// Render the PR dashboard screen.
pub fn render(state: &PrDashboardState, frame: &mut Frame, area: Rect) {
    if state.detail_view {
        render_detail(state, frame, area);
    } else {
        render_list(state, frame, area);
    }
}

/// Render the PR list.
fn render_list(state: &PrDashboardState, frame: &mut Frame, area: Rect) {
    if state.prs.is_empty() {
        let block = Block::default().title("PR Dashboard");
        let paragraph = Paragraph::new("No pull requests loaded")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let title = format!("PR Dashboard ({})", state.prs.len());
    let items: Vec<ListItem> = state
        .prs
        .iter()
        .enumerate()
        .map(|(idx, pr)| {
            let style = super::list_item_style(idx == state.selected);

            let ci_color = match pr.ci_status.as_str() {
                "pass" | "success" => Color::Green,
                "fail" | "failure" => Color::Red,
                "pending" => Color::Yellow,
                _ => Color::DarkGray,
            };

            let merge_indicator = if pr.mergeable { "OK" } else { "!!" };
            let merge_color = if pr.mergeable {
                Color::Green
            } else {
                Color::Red
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("#{:<5} ", pr.number),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("{:<8} ", pr.state.label()),
                    Style::default().fg(pr.state.color()),
                ),
                Span::styled(pr.title.clone(), style),
                Span::styled(
                    format!("  CI:{}", pr.ci_status),
                    Style::default().fg(ci_color),
                ),
                Span::styled(
                    format!(" Merge:{merge_indicator}"),
                    Style::default().fg(merge_color),
                ),
                Span::styled(
                    format!(" Review:{}", pr.review_status),
                    Style::default().fg(Color::Cyan),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().title(title);
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Render the detail view for the selected PR.
fn render_detail(state: &PrDashboardState, frame: &mut Frame, area: Rect) {
    let pr = match state.selected_pr() {
        Some(p) => p,
        None => {
            let block = Block::default().title("PR Detail");
            let paragraph = Paragraph::new("No PR selected")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
            return;
        }
    };

    let merge_str = if pr.mergeable {
        "Yes"
    } else {
        "No (conflicts)"
    };

    let detail_text =
        format!(
        " #{} - {}\n\n State: {}\n CI: {}\n Mergeable: {}\n Review: {}\n\n Press Enter to go back",
        pr.number, pr.title, pr.state.label(), pr.ci_status, merge_str, pr.review_status,
    );

    let block = Block::default().title("PR Detail");
    let paragraph = Paragraph::new(detail_text)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_prs() -> Vec<PrItem> {
        vec![
            PrItem {
                number: 101,
                title: "Add login feature".to_string(),
                state: PrState::Open,
                ci_status: "pass".to_string(),
                mergeable: true,
                review_status: "approved".to_string(),
            },
            PrItem {
                number: 102,
                title: "Fix database issue".to_string(),
                state: PrState::Open,
                ci_status: "fail".to_string(),
                mergeable: false,
                review_status: "changes_requested".to_string(),
            },
            PrItem {
                number: 100,
                title: "Update docs".to_string(),
                state: PrState::Merged,
                ci_status: "success".to_string(),
                mergeable: true,
                review_status: "approved".to_string(),
            },
            PrItem {
                number: 99,
                title: "Remove old API".to_string(),
                state: PrState::Closed,
                ci_status: "pending".to_string(),
                mergeable: false,
                review_status: "none".to_string(),
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = PrDashboardState::default();
        assert!(state.prs.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.detail_view);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = PrDashboardState::default();
        state.prs = sample_prs();

        update(&mut state, PrDashboardMessage::MoveDown);
        assert_eq!(state.selected, 1);

        for _ in 0..3 {
            update(&mut state, PrDashboardMessage::MoveDown);
        }
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = PrDashboardState::default();
        state.prs = sample_prs();

        update(&mut state, PrDashboardMessage::MoveUp);
        assert_eq!(state.selected, 3); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = PrDashboardState::default();
        update(&mut state, PrDashboardMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, PrDashboardMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn toggle_detail_flips() {
        let mut state = PrDashboardState::default();
        state.prs = sample_prs();
        assert!(!state.detail_view);

        update(&mut state, PrDashboardMessage::ToggleDetail);
        assert!(state.detail_view);

        update(&mut state, PrDashboardMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn toggle_detail_noop_on_empty() {
        let mut state = PrDashboardState::default();
        update(&mut state, PrDashboardMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn set_prs_populates() {
        let mut state = PrDashboardState::default();
        state.selected = 99;
        update(&mut state, PrDashboardMessage::SetPrs(sample_prs()));
        assert_eq!(state.prs.len(), 4);
        assert_eq!(state.selected, 3); // clamped
    }

    #[test]
    fn selected_pr_returns_correct() {
        let mut state = PrDashboardState::default();
        state.prs = sample_prs();
        state.selected = 2;
        let pr = state.selected_pr().unwrap();
        assert_eq!(pr.number, 100);
    }

    #[test]
    fn pr_state_labels() {
        assert_eq!(PrState::Open.label(), "Open");
        assert_eq!(PrState::Closed.label(), "Closed");
        assert_eq!(PrState::Merged.label(), "Merged");
    }

    #[test]
    fn pr_state_colors() {
        assert_eq!(PrState::Open.color(), Color::Green);
        assert_eq!(PrState::Closed.color(), Color::Red);
        assert_eq!(PrState::Merged.color(), Color::Magenta);
    }

    #[test]
    fn render_list_does_not_panic() {
        let mut state = PrDashboardState::default();
        state.prs = sample_prs();
        let backend = TestBackend::new(100, 24);
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
        assert!(text.contains("PR Dashboard"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = PrDashboardState::default();
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
        let mut state = PrDashboardState::default();
        state.prs = sample_prs();
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
    fn render_detail_no_selection_does_not_panic() {
        let mut state = PrDashboardState::default();
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
    fn set_prs_empty_clamps() {
        let mut state = PrDashboardState::default();
        state.prs = sample_prs();
        state.selected = 3;

        update(&mut state, PrDashboardMessage::SetPrs(vec![]));
        assert!(state.prs.is_empty());
        assert_eq!(state.selected, 0);
    }
}
