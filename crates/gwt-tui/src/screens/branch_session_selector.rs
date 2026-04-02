//! Branch session selector overlay for `many sessions` branch enter flow.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchSessionSelectorChoice {
    ExistingSession(usize),
    AddSession,
    FullWizard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSessionOption {
    pub label: String,
    pub choice: BranchSessionSelectorChoice,
}

#[derive(Debug, Clone)]
pub struct BranchSessionSelectorState {
    pub branch_name: String,
    pub worktree_path: Option<String>,
    pub options: Vec<BranchSessionOption>,
    pub selected: usize,
}

impl BranchSessionSelectorState {
    pub fn new(
        branch_name: impl Into<String>,
        worktree_path: Option<String>,
        options: Vec<BranchSessionOption>,
    ) -> Self {
        Self {
            branch_name: branch_name.into(),
            worktree_path,
            options,
            selected: 0,
        }
    }

    pub fn select_next(&mut self) {
        if self.options.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.options.len() - 1);
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn selected_choice(&self) -> Option<&BranchSessionSelectorChoice> {
        self.options.get(self.selected).map(|option| &option.choice)
    }
}

pub fn render(state: &BranchSessionSelectorState, buf: &mut Buffer, area: Rect) {
    let width = 60.min(area.width.saturating_sub(4));
    let height = (state.options.len() as u16 + 6).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    Clear.render(dialog_area, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(format!(" Sessions — {} ", state.branch_name));
    let inner = block.inner(dialog_area);
    block.render(dialog_area, buf);

    let header =
        Paragraph::new("Select an existing session, add a new one, or open the full wizard.")
            .style(Style::default().fg(Color::Gray));
    header.render(Rect::new(inner.x, inner.y, inner.width, 2), buf);

    for (index, option) in state.options.iter().enumerate() {
        let y = inner.y + 2 + index as u16;
        if y >= inner.bottom() {
            break;
        }
        let prefix = if index == state.selected { ">" } else { " " };
        let style = if index == state.selected {
            Style::default().fg(Color::Black).bg(Color::Cyan).bold()
        } else {
            Style::default().fg(Color::White)
        };
        let line = Line::from(Span::styled(format!(" {prefix} {}", option.label), style));
        buf.set_line(inner.x, y, &line, inner.width);
    }

    let footer = Paragraph::new("[↑/↓] Select  [Enter] Confirm  [Esc] Cancel")
        .style(Style::default().fg(Color::DarkGray));
    let footer_y = inner.bottom().saturating_sub(1);
    footer.render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_navigation_clamps() {
        let mut state = BranchSessionSelectorState::new(
            "feature/demo",
            None,
            vec![
                BranchSessionOption {
                    label: "Session A".into(),
                    choice: BranchSessionSelectorChoice::ExistingSession(0),
                },
                BranchSessionOption {
                    label: "Add".into(),
                    choice: BranchSessionSelectorChoice::AddSession,
                },
            ],
        );

        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_prev();
        assert_eq!(state.selected, 0);
        state.select_prev();
        assert_eq!(state.selected, 0);
    }
}
