//! Service selection overlay screen.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

/// State for the service selection overlay.
#[derive(Debug, Clone, Default)]
pub struct ServiceSelectState {
    pub title: String,
    pub services: Vec<String>,
    pub values: Vec<String>,
    pub selected: usize,
    pub visible: bool,
}

impl ServiceSelectState {
    pub fn with_options(
        title: impl Into<String>,
        services: Vec<String>,
        values: Vec<String>,
    ) -> Self {
        debug_assert_eq!(services.len(), values.len());
        Self {
            title: title.into(),
            services,
            values,
            selected: 0,
            visible: true,
        }
    }

    pub fn current_selection(&self) -> Option<(&str, &str)> {
        let service = self.services.get(self.selected)?;
        let value = self.values.get(self.selected)?;
        Some((service.as_str(), value.as_str()))
    }
}

/// Messages for the service selection overlay.
#[derive(Debug, Clone)]
pub enum ServiceSelectMessage {
    MoveUp,
    MoveDown,
    Select,
    Cancel,
    SetServices(Vec<String>),
}

/// Update service selection state.
pub fn update(state: &mut ServiceSelectState, msg: ServiceSelectMessage) {
    match msg {
        ServiceSelectMessage::MoveUp => {
            if !state.services.is_empty() && state.selected > 0 {
                state.selected -= 1;
            }
        }
        ServiceSelectMessage::MoveDown => {
            if !state.services.is_empty() && state.selected + 1 < state.services.len() {
                state.selected += 1;
            }
        }
        ServiceSelectMessage::Select => {
            state.visible = false;
        }
        ServiceSelectMessage::Cancel => {
            state.visible = false;
        }
        ServiceSelectMessage::SetServices(services) => {
            state.values = services.clone();
            state.services = services;
            state.selected = 0;
        }
    }
}

/// Render the service selection overlay.
pub fn render(state: &ServiceSelectState, frame: &mut Frame, area: Rect) {
    if !state.visible {
        return;
    }

    let width = 50_u16.min(area.width);
    let height = (state.services.len() as u16 + 4).min(area.height).max(6);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog = Rect::new(x, y, width, height);

    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(if state.title.is_empty() {
            "Select Service"
        } else {
            state.title.as_str()
        })
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    if state.services.is_empty() {
        let empty = Paragraph::new("No services found").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .services
        .iter()
        .enumerate()
        .map(|(i, svc)| {
            let style = if i == state.selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let icon = if i == state.selected {
                "\u{25B6} "
            } else {
                "  "
            };
            ListItem::new(Line::from(Span::styled(format!("{icon}{svc}"), style)))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_services() -> Vec<String> {
        vec![
            "web".to_string(),
            "api".to_string(),
            "db".to_string(),
            "redis".to_string(),
        ]
    }

    #[test]
    fn default_state() {
        let state = ServiceSelectState::default();
        assert!(state.services.is_empty());
        assert!(state.values.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.visible);
    }

    #[test]
    fn set_services_resets_selection() {
        let mut state = ServiceSelectState {
            selected: 2,
            ..ServiceSelectState::default()
        };
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );
        assert_eq!(state.services.len(), 4);
        assert_eq!(state.values, sample_services());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_down_increments_selection() {
        let mut state = ServiceSelectState::default();
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );

        update(&mut state, ServiceSelectMessage::MoveDown);
        assert_eq!(state.selected, 1);

        update(&mut state, ServiceSelectMessage::MoveDown);
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn move_down_clamps_at_end() {
        let mut state = ServiceSelectState::default();
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );
        for _ in 0..10 {
            update(&mut state, ServiceSelectMessage::MoveDown);
        }
        assert_eq!(state.selected, 3); // last index
    }

    #[test]
    fn move_up_decrements_selection() {
        let mut state = ServiceSelectState::default();
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );
        state.selected = 2;

        update(&mut state, ServiceSelectMessage::MoveUp);
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn move_up_clamps_at_zero() {
        let mut state = ServiceSelectState::default();
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );
        update(&mut state, ServiceSelectMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn select_hides_overlay() {
        let mut state = ServiceSelectState {
            title: "Select Service".into(),
            visible: true,
            ..ServiceSelectState::default()
        };
        update(&mut state, ServiceSelectMessage::Select);
        assert!(!state.visible);
    }

    #[test]
    fn cancel_hides_overlay() {
        let mut state = ServiceSelectState {
            title: "Select Service".into(),
            visible: true,
            ..ServiceSelectState::default()
        };
        update(&mut state, ServiceSelectMessage::Cancel);
        assert!(!state.visible);
    }

    #[test]
    fn render_visible_does_not_panic() {
        let mut state = ServiceSelectState {
            visible: true,
            ..ServiceSelectState::default()
        };
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );
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
        assert!(full_text.contains("Select Service"));
    }

    #[test]
    fn render_invisible_is_noop() {
        let state = ServiceSelectState::default();
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
        assert!(!full_text.contains("Select Service"));
    }
}
