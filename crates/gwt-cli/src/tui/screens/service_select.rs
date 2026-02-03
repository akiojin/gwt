//! Service Selection Screen (SPEC-f5f5657e)
//!
//! Allows users to select which Docker service to use when multiple services
//! are defined in a docker-compose.yml file.

use ratatui::{prelude::*, widgets::*};

/// Service selection screen state
#[derive(Debug)]
pub struct ServiceSelectState {
    /// List of available services
    pub services: Vec<String>,
    /// Currently selected index
    pub selected: usize,
    /// Container name for context
    pub container_name: String,
    /// Worktree name for context
    pub worktree_name: String,
    // Mouse click support
    /// Cached popup area
    pub popup_area: Option<Rect>,
    /// Cached list item areas for click detection
    pub item_areas: Vec<Rect>,
}

impl Default for ServiceSelectState {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceSelectState {
    /// Create a new ServiceSelectState
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
            selected: 0,
            container_name: String::new(),
            worktree_name: String::new(),
            popup_area: None,
            item_areas: Vec::new(),
        }
    }

    /// Create a ServiceSelectState with services
    pub fn with_services(services: Vec<String>) -> Self {
        Self {
            services,
            selected: 0,
            container_name: String::new(),
            worktree_name: String::new(),
            popup_area: None,
            item_areas: Vec::new(),
        }
    }

    /// Set container information
    pub fn set_container_info(&mut self, container_name: &str, worktree_name: &str) {
        self.container_name = container_name.to_string();
        self.worktree_name = worktree_name.to_string();
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if !self.services.is_empty() {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.services.is_empty() {
            self.selected = (self.selected + 1).min(self.services.len().saturating_sub(1));
        }
    }

    /// Get the currently selected service
    pub fn selected_service(&self) -> Option<&str> {
        self.services.get(self.selected).map(|s| s.as_str())
    }

    /// Check if there are multiple services (selection needed)
    pub fn needs_selection(&self) -> bool {
        self.services.len() > 1
    }

    /// Handle click on a service item
    pub fn handle_click(&mut self, x: u16, y: u16) -> bool {
        for (i, area) in self.item_areas.iter().enumerate() {
            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                self.selected = i;
                return true;
            }
        }
        false
    }
}

/// Render the service selection screen
pub fn render_service_select(state: &mut ServiceSelectState, frame: &mut Frame, area: Rect) {
    // Calculate popup size based on number of services
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = (state.services.len() as u16 + 8).min(area.height.saturating_sub(4));

    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    // Store popup area for click detection
    state.popup_area = Some(popup_area);

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Outer block
    let block = Block::default()
        .title(" Select Service ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .border_type(BorderType::Rounded);

    frame.render_widget(block, popup_area);

    // Inner area for content
    let inner = Rect {
        x: popup_area.x + 2,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(4),
        height: popup_area.height.saturating_sub(2),
    };

    // Layout for header and list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(1),    // Services list
            Constraint::Length(2), // Footer
        ])
        .split(inner);

    // Header with context
    let header_lines = vec![
        Line::from(vec![
            Span::styled("Worktree: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.worktree_name, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Container: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.container_name, Style::default().fg(Color::Yellow)),
        ]),
    ];
    let header = Paragraph::new(header_lines);
    frame.render_widget(header, chunks[0]);

    // Services list
    let items: Vec<ListItem> = state
        .services
        .iter()
        .enumerate()
        .map(|(i, service)| {
            let style = if i == state.selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if i == state.selected { "> " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(service, style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan));

    frame.render_widget(list, chunks[1]);

    // Store item areas for click detection
    state.item_areas.clear();
    for i in 0..state.services.len() {
        if (i as u16) < chunks[1].height {
            state.item_areas.push(Rect {
                x: chunks[1].x,
                y: chunks[1].y + i as u16,
                width: chunks[1].width,
                height: 1,
            });
        }
    }

    // Footer with instructions
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("[Up/Down]", Style::default().fg(Color::DarkGray)),
        Span::raw(" Navigate  "),
        Span::styled("[Enter]", Style::default().fg(Color::DarkGray)),
        Span::raw(" Select  "),
        Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
        Span::raw(" Cancel"),
    ]))
    .alignment(Alignment::Center);

    frame.render_widget(footer, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-402: Service selection UI test
    #[test]
    fn test_service_select_state_new() {
        let state = ServiceSelectState::new();
        assert!(state.services.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.needs_selection());
    }

    #[test]
    fn test_service_select_state_with_services() {
        let services = vec!["web".to_string(), "db".to_string(), "redis".to_string()];
        let state = ServiceSelectState::with_services(services.clone());
        assert_eq!(state.services.len(), 3);
        assert!(state.needs_selection());
        assert_eq!(state.selected_service(), Some("web"));
    }

    // T-403: Keyboard operation test
    #[test]
    fn test_service_select_navigation() {
        let services = vec!["web".to_string(), "db".to_string(), "redis".to_string()];
        let mut state = ServiceSelectState::with_services(services);

        // Initial selection is first item
        assert_eq!(state.selected, 0);
        assert_eq!(state.selected_service(), Some("web"));

        // Move down
        state.select_next();
        assert_eq!(state.selected, 1);
        assert_eq!(state.selected_service(), Some("db"));

        // Move down again
        state.select_next();
        assert_eq!(state.selected, 2);
        assert_eq!(state.selected_service(), Some("redis"));

        // Try to move past the end (should stay at last)
        state.select_next();
        assert_eq!(state.selected, 2);

        // Move up
        state.select_previous();
        assert_eq!(state.selected, 1);

        // Move up again
        state.select_previous();
        assert_eq!(state.selected, 0);

        // Try to move past the start (should stay at first)
        state.select_previous();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_service_select_single_service() {
        let services = vec!["web".to_string()];
        let state = ServiceSelectState::with_services(services);
        assert!(!state.needs_selection()); // Single service, no selection needed
        assert_eq!(state.selected_service(), Some("web"));
    }

    #[test]
    fn test_service_select_empty() {
        let state = ServiceSelectState::new();
        assert!(!state.needs_selection());
        assert_eq!(state.selected_service(), None);
    }

    #[test]
    fn test_service_select_set_container_info() {
        let mut state = ServiceSelectState::new();
        state.set_container_info("gwt-my-worktree", "my-worktree");
        assert_eq!(state.container_name, "gwt-my-worktree");
        assert_eq!(state.worktree_name, "my-worktree");
    }
}
