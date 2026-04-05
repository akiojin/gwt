//! Port conflict resolution overlay screen.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

/// A single port conflict entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortConflict {
    pub container_port: u16,
    pub host_port: u16,
    pub suggested: u16,
}

/// State for the port selection overlay.
#[derive(Debug, Clone, Default)]
pub struct PortSelectState {
    pub conflicts: Vec<PortConflict>,
    pub selected: usize,
    pub visible: bool,
}

impl PortSelectState {
    /// Create a visible port selector only when conflicts exist.
    pub fn with_conflicts(conflicts: Vec<PortConflict>) -> Self {
        let visible = !conflicts.is_empty();
        Self {
            conflicts,
            selected: 0,
            visible,
        }
    }

    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    pub fn is_resolved(&self) -> bool {
        self.conflicts
            .iter()
            .all(|conflict| conflict.host_port == conflict.suggested)
    }
}

/// Messages for the port selection overlay.
#[derive(Debug, Clone)]
pub enum PortSelectMessage {
    MoveUp,
    MoveDown,
    /// Accept the suggested port for the selected conflict.
    Accept,
    /// Accept all suggested ports.
    AcceptAll,
    Cancel,
}

/// Update port selection state.
pub fn update(state: &mut PortSelectState, msg: PortSelectMessage) {
    match msg {
        PortSelectMessage::MoveUp => {
            if !state.conflicts.is_empty() && state.selected > 0 {
                state.selected -= 1;
            }
        }
        PortSelectMessage::MoveDown => {
            if !state.conflicts.is_empty() && state.selected + 1 < state.conflicts.len() {
                state.selected += 1;
            }
        }
        PortSelectMessage::Accept => {
            if let Some(conflict) = state.conflicts.get_mut(state.selected) {
                conflict.host_port = conflict.suggested;
            }
            state.visible = !state.is_resolved();
        }
        PortSelectMessage::AcceptAll => {
            for conflict in &mut state.conflicts {
                conflict.host_port = conflict.suggested;
            }
            state.visible = !state.is_resolved();
        }
        PortSelectMessage::Cancel => {
            state.visible = false;
        }
    }
}

/// Render the port selection overlay.
pub fn render(state: &PortSelectState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    let width = 56_u16.min(area.width);
    let height = (state.conflicts.len() as u16 + 5).min(area.height).max(7);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Port Conflicts")
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    if state.conflicts.is_empty() {
        let empty = Paragraph::new("No port conflicts").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, inner);
        return;
    }

    // Header
    if inner.height < 2 {
        return;
    }
    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let header = Paragraph::new(Line::from(vec![Span::styled(
        "Container  Host  Suggested",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    frame.render_widget(header, header_area);

    let list_area = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );

    let items: Vec<ListItem> = state
        .conflicts
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let resolved = c.host_port == c.suggested;
            let style = if i == state.selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else if resolved {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            let icon = if resolved { "\u{2714}" } else { "\u{26A0}" };
            ListItem::new(Line::from(Span::styled(
                format!(
                    "{icon} :{:<6} :{:<6} :{:<6}",
                    c.container_port, c.host_port, c.suggested
                ),
                style,
            )))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, list_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_conflicts() -> Vec<PortConflict> {
        vec![
            PortConflict {
                container_port: 8080,
                host_port: 8080,
                suggested: 8081,
            },
            PortConflict {
                container_port: 5432,
                host_port: 5432,
                suggested: 5433,
            },
            PortConflict {
                container_port: 6379,
                host_port: 6379,
                suggested: 6380,
            },
        ]
    }

    fn single_conflict() -> Vec<PortConflict> {
        vec![PortConflict {
            container_port: 3000,
            host_port: 3000,
            suggested: 3001,
        }]
    }

    #[test]
    fn default_state() {
        let state = PortSelectState::default();
        assert!(state.conflicts.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.visible);
    }

    #[test]
    fn with_conflicts_marks_visible_and_detects_conflicts() {
        let state = PortSelectState::with_conflicts(sample_conflicts());
        assert!(state.visible);
        assert!(state.has_conflicts());
        assert!(!state.is_resolved());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn with_conflicts_empty_auto_hides_overlay() {
        let state = PortSelectState::with_conflicts(vec![]);
        assert!(!state.visible);
        assert!(!state.has_conflicts());
        assert!(state.is_resolved());
    }

    #[test]
    fn move_down_increments() {
        let mut state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 0,
            visible: true,
        };

        update(&mut state, PortSelectMessage::MoveDown);
        assert_eq!(state.selected, 1);

        update(&mut state, PortSelectMessage::MoveDown);
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn move_down_clamps_at_end() {
        let mut state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 0,
            visible: true,
        };
        for _ in 0..10 {
            update(&mut state, PortSelectMessage::MoveDown);
        }
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn move_up_decrements() {
        let mut state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 2,
            visible: true,
        };

        update(&mut state, PortSelectMessage::MoveUp);
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn move_up_clamps_at_zero() {
        let mut state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 0,
            visible: true,
        };
        update(&mut state, PortSelectMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn accept_applies_suggested_to_selected() {
        let mut state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 0,
            visible: true,
        };
        assert_eq!(state.conflicts[0].host_port, 8080);

        update(&mut state, PortSelectMessage::Accept);
        assert_eq!(state.conflicts[0].host_port, 8081);
        // Others unchanged
        assert_eq!(state.conflicts[1].host_port, 5432);
    }

    #[test]
    fn accept_all_applies_suggested_to_all() {
        let mut state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 0,
            visible: true,
        };

        update(&mut state, PortSelectMessage::AcceptAll);
        assert_eq!(state.conflicts[0].host_port, 8081);
        assert_eq!(state.conflicts[1].host_port, 5433);
        assert_eq!(state.conflicts[2].host_port, 6380);
    }

    #[test]
    fn accept_closes_when_last_conflict_is_resolved() {
        let mut state = PortSelectState::with_conflicts(single_conflict());
        assert!(state.visible);

        update(&mut state, PortSelectMessage::Accept);

        assert_eq!(state.conflicts[0].host_port, 3001);
        assert!(!state.visible);
        assert!(state.is_resolved());
    }

    #[test]
    fn accept_all_closes_overlay_after_resolving_everything() {
        let mut state = PortSelectState::with_conflicts(sample_conflicts());
        assert!(state.visible);

        update(&mut state, PortSelectMessage::AcceptAll);

        assert_eq!(state.conflicts[0].host_port, 8081);
        assert_eq!(state.conflicts[1].host_port, 5433);
        assert_eq!(state.conflicts[2].host_port, 6380);
        assert!(!state.visible);
        assert!(state.is_resolved());
    }

    #[test]
    fn cancel_hides_overlay() {
        let mut state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 0,
            visible: true,
        };
        update(&mut state, PortSelectMessage::Cancel);
        assert!(!state.visible);
    }

    #[test]
    fn render_visible_does_not_panic() {
        let state = PortSelectState {
            conflicts: sample_conflicts(),
            selected: 1,
            visible: true,
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(full_text.contains("Port Conflicts"));
    }

    #[test]
    fn render_invisible_is_noop() {
        let state = PortSelectState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let full_text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(!full_text.contains("Port Conflicts"));
    }
}
