//! Service Selection Screen (SPEC-f5f5657e)
//!
//! Allows users to select which Docker service to use when multiple services
//! are defined in a docker-compose.yml file.

use ratatui::{prelude::*, widgets::*};

#[derive(Debug, Clone)]
pub struct ServiceSelectItem {
    pub label: String,
    pub service: Option<String>,
    pub is_host: bool,
}

/// Service selection screen state
#[derive(Debug)]
pub struct ServiceSelectState {
    /// List of available services
    pub items: Vec<ServiceSelectItem>,
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
            items: Vec::new(),
            selected: 0,
            container_name: String::new(),
            worktree_name: String::new(),
            popup_area: None,
            item_areas: Vec::new(),
        }
    }

    /// Create a ServiceSelectState with services
    pub fn with_services(services: Vec<String>) -> Self {
        let mut items = Vec::with_capacity(services.len().saturating_add(1));
        items.push(ServiceSelectItem {
            label: "HostOS".to_string(),
            service: None,
            is_host: true,
        });
        for service in services.iter() {
            items.push(ServiceSelectItem {
                label: format!("Docker:{}", service),
                service: Some(service.clone()),
                is_host: false,
            });
        }
        Self {
            items,
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
        if !self.items.is_empty() {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
        }
    }

    pub fn selected_target(&self) -> (Option<&str>, bool) {
        match self.items.get(self.selected) {
            Some(item) if item.is_host => (None, true),
            Some(item) => (item.service.as_deref(), false),
            None => (None, false),
        }
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
    let popup_height = (state.items.len() as u16 + 8).min(area.height.saturating_sub(4));

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
    let list_width = chunks[1].width.saturating_sub(1) as usize;
    let items: Vec<ListItem> = state
        .items
        .iter()
        .map(|item| {
            let mut label = item.label.clone();
            if label.len() < list_width {
                label.push_str(&" ".repeat(list_width - label.len()));
            }
            ListItem::new(Line::from(label))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan));

    let mut list_state = ListState::default();
    if !state.items.is_empty() {
        list_state.select(Some(state.selected));
    }
    frame.render_stateful_widget(list, chunks[1], &mut list_state);

    // Store item areas for click detection
    state.item_areas.clear();
    for i in 0..state.items.len() {
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
        Span::styled("[S]", Style::default().fg(Color::DarkGray)),
        Span::raw(" Skip  "),
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
        assert!(state.items.is_empty());
        assert_eq!(state.selected, 0);
        assert!(state.items.len() <= 1);
    }

    #[test]
    fn test_service_select_state_with_services() {
        let services = vec!["web".to_string(), "db".to_string(), "redis".to_string()];
        let state = ServiceSelectState::with_services(services.clone());
        assert_eq!(state.items.len(), 4);
        assert!(state.items.len() > 1);
        assert_eq!(state.selected_target(), (None, true));
        assert_eq!(state.items[0].label, "HostOS");
        assert_eq!(state.items[1].label, "Docker:web");
    }

    // T-403: Keyboard operation test
    #[test]
    fn test_service_select_navigation() {
        let services = vec!["web".to_string(), "db".to_string(), "redis".to_string()];
        let mut state = ServiceSelectState::with_services(services);

        // Initial selection is first item
        assert_eq!(state.selected, 0);
        assert_eq!(state.selected_target(), (None, true));

        // Move down
        state.select_next();
        assert_eq!(state.selected, 1);
        assert_eq!(state.selected_target(), (Some("web"), false));

        // Move down again
        state.select_next();
        assert_eq!(state.selected, 2);
        assert_eq!(state.selected_target(), (Some("db"), false));

        // Try to move past the end (should stay at last)
        state.select_next();
        assert_eq!(state.selected, 3);

        // Move up
        state.select_previous();
        assert_eq!(state.selected, 2);

        // Move up again
        state.select_previous();
        assert_eq!(state.selected, 1);

        // Try to move past the start (should stay at first)
        state.select_previous();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_service_select_single_service() {
        let services = vec!["web".to_string()];
        let state = ServiceSelectState::with_services(services);
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.selected_target(), (None, true));
    }

    #[test]
    fn test_service_select_empty() {
        let state = ServiceSelectState::new();
        assert!(state.items.is_empty());
        assert_eq!(state.selected_target(), (None, false));
    }

    #[test]
    fn test_service_select_target_host_and_docker() {
        let services = vec!["web".to_string()];
        let mut state = ServiceSelectState::with_services(services);
        assert_eq!(state.selected_target(), (None, true));
        state.select_next();
        assert_eq!(state.selected_target(), (Some("web"), false));
    }

    #[test]
    fn test_service_select_set_container_info() {
        let mut state = ServiceSelectState::new();
        state.set_container_info("gwt-my-worktree", "my-worktree");
        assert_eq!(state.container_name, "gwt-my-worktree");
        assert_eq!(state.worktree_name, "my-worktree");
    }
}
