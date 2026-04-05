//! Service selection overlay screen.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::theme;

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
    fn should_show_overlay(services: &[String]) -> bool {
        services.len() != 1
    }

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
            state.visible = ServiceSelectState::should_show_overlay(&state.services);
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
        .border_type(theme::border::default())
        .border_style(Style::default().fg(theme::color::FOCUS));

    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    if state.services.is_empty() {
        let empty = Paragraph::new("No services found").style(theme::style::muted_text());
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
                    .fg(theme::color::TEXT_PRIMARY)
                    .bg(theme::color::AGENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::color::TEXT_PRIMARY)
            };
            let icon = if i == state.selected {
                concat!("\u{25B6}", " ") // theme::icon::ARROW_RIGHT + space
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

    fn single_service() -> Vec<String> {
        vec!["web".to_string()]
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect()
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
    fn set_services_shows_overlay_for_multiple_services() {
        let mut state = ServiceSelectState::default();
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );

        assert!(state.visible);
        assert_eq!(state.selected, 0);
        assert_eq!(state.current_selection(), Some(("web", "web")));
    }

    #[test]
    fn set_services_keeps_error_overlay_visible_when_empty() {
        let mut state = ServiceSelectState::default();
        update(&mut state, ServiceSelectMessage::SetServices(Vec::new()));

        assert!(state.visible);
        assert!(state.current_selection().is_none());
    }

    #[test]
    fn set_services_auto_selects_single_service() {
        let mut state = ServiceSelectState {
            visible: true,
            ..ServiceSelectState::default()
        };
        update(
            &mut state,
            ServiceSelectMessage::SetServices(single_service()),
        );

        assert!(!state.visible);
        assert_eq!(state.current_selection(), Some(("web", "web")));
    }

    #[test]
    fn with_options_keeps_overlay_visible_for_single_option() {
        let state = ServiceSelectState::with_options(
            "Select Agent",
            single_service(),
            vec!["web".to_string()],
        );

        assert!(state.visible);
        assert_eq!(state.current_selection(), Some(("web", "web")));
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
        assert!(buffer_text(&terminal).contains("Select Service"));
    }

    #[test]
    fn render_lists_all_services_and_selection_marker() {
        let mut state = ServiceSelectState {
            visible: true,
            ..ServiceSelectState::default()
        };
        update(
            &mut state,
            ServiceSelectMessage::SetServices(sample_services()),
        );
        state.visible = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let full_text = buffer_text(&terminal);
        assert!(full_text.contains("web"));
        assert!(full_text.contains("api"));
        assert!(full_text.contains("db"));
        assert!(full_text.contains("redis"));
        assert!(full_text.contains("▶"));
    }

    #[test]
    fn render_empty_state_shows_error_message() {
        let state = ServiceSelectState {
            visible: true,
            ..ServiceSelectState::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        assert!(buffer_text(&terminal).contains("No services found"));
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
        assert!(!buffer_text(&terminal).contains("Select Service"));
    }
}
