//! Profile Management Screen

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};

/// Profile item
#[derive(Debug, Clone)]
pub struct ProfileItem {
    /// Profile name
    pub name: String,
    /// Is this the active profile
    pub is_active: bool,
    /// Environment variables count
    pub env_count: usize,
    /// Description (optional)
    pub description: Option<String>,
}

/// Profile management state
#[derive(Debug, Default)]
pub struct ProfilesState {
    /// Available profiles
    pub profiles: Vec<ProfileItem>,
    /// Currently selected index
    pub selected: usize,
    /// Active profile name
    pub active_profile: Option<String>,
    /// Is in create mode
    pub create_mode: bool,
    /// New profile name input
    pub new_name: String,
    /// Cursor position for input
    pub cursor: usize,
    /// Error message
    pub error: Option<String>,
}

impl ProfilesState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize with profiles
    pub fn with_profiles(mut self, profiles: Vec<ProfileItem>) -> Self {
        self.profiles = profiles;
        self.active_profile = self
            .profiles
            .iter()
            .find(|p| p.is_active)
            .map(|p| p.name.clone());
        self
    }

    /// Get selected profile
    pub fn selected_profile(&self) -> Option<&ProfileItem> {
        self.profiles.get(self.selected)
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if !self.create_mode && self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.create_mode && self.selected < self.profiles.len().saturating_sub(1) {
            self.selected += 1;
        }
    }

    /// Enter create mode
    pub fn enter_create_mode(&mut self) {
        self.create_mode = true;
        self.new_name.clear();
        self.cursor = 0;
        self.error = None;
    }

    /// Exit create mode
    pub fn exit_create_mode(&mut self) {
        self.create_mode = false;
        self.new_name.clear();
        self.cursor = 0;
    }

    /// Insert character in create mode
    pub fn insert_char(&mut self, c: char) {
        if self.create_mode {
            self.new_name.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Delete character in create mode
    pub fn delete_char(&mut self) {
        if self.create_mode && self.cursor > 0 {
            self.cursor -= 1;
            self.new_name.remove(self.cursor);
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if self.cursor < self.new_name.len() {
            self.cursor += 1;
        }
    }

    /// Validate and get new profile name
    pub fn validate_new_name(&self) -> Result<String, &'static str> {
        let name = self.new_name.trim();
        if name.is_empty() {
            return Err("Profile name cannot be empty");
        }
        if self.profiles.iter().any(|p| p.name == name) {
            return Err("Profile with this name already exists");
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err("Profile name can only contain lowercase letters, numbers, and hyphens");
        }
        Ok(name.to_string())
    }
}

/// Render profiles screen
pub fn render_profiles(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    // Layout depends on create_mode
    let chunks = if state.create_mode {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(5),    // Profile list
                Constraint::Length(3), // Input area
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Profile list (takes all remaining space)
            ])
            .split(area)
    };

    // Header
    let header_text = format!(
        "Profiles ({}) | Active: {}",
        state.profiles.len(),
        state.active_profile.as_deref().unwrap_or("none")
    );
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, chunks[0]);

    // Profile list
    if state.profiles.is_empty() {
        let empty = Paragraph::new("No profiles found. Press 'n' to create one.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = state
            .profiles
            .iter()
            .enumerate()
            .map(|(i, profile)| {
                let active_marker = if profile.is_active { "*" } else { " " };
                let env_info = format!("[{} vars]", profile.env_count);
                let desc = profile.description.as_deref().unwrap_or("");

                let line = Line::from(vec![
                    Span::styled(
                        format!("{} ", active_marker),
                        Style::default().fg(if profile.is_active {
                            Color::Green
                        } else {
                            Color::DarkGray
                        }),
                    ),
                    Span::raw(&profile.name),
                    Span::styled(
                        format!(" {} ", env_info),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(desc, Style::default().fg(Color::DarkGray)),
                ]);

                let style = if i == state.selected && !state.create_mode {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default()
                };

                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::NONE));
        frame.render_widget(list, chunks[1]);
    }

    // Input area (only in create_mode)
    if state.create_mode {
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" New Profile Name ");

        let input_inner = input_block.inner(chunks[2]);
        frame.render_widget(input_block, chunks[2]);

        let input_text = if state.new_name.is_empty() {
            "Enter profile name...".to_string()
        } else {
            state.new_name.clone()
        };

        let input_style = if state.new_name.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };

        let input = Paragraph::new(input_text).style(input_style);
        frame.render_widget(input, input_inner);

        // Show cursor
        if !state.new_name.is_empty() {
            frame.set_cursor_position((input_inner.x + state.cursor as u16, input_inner.y));
        }
    }

    // Show error if any
    if let Some(error) = &state.error {
        let error_area = Rect {
            x: area.x + 2,
            y: area.y + area.height - 2,
            width: area.width - 4,
            height: 1,
        };
        let error_msg = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_msg, error_area);
    }
}

fn profile_actions_text() -> &'static str {
    "[Space] Activate | [Enter] Edit env | [n] New | [d] Delete | [Esc] Back"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_navigation() {
        let profiles = vec![
            ProfileItem {
                name: "default".to_string(),
                is_active: true,
                env_count: 5,
                description: None,
            },
            ProfileItem {
                name: "development".to_string(),
                is_active: false,
                env_count: 10,
                description: Some("Dev environment".to_string()),
            },
        ];

        let mut state = ProfilesState::new().with_profiles(profiles);
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_next();
        assert_eq!(state.selected, 1); // Should not go beyond

        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_create_mode() {
        let mut state = ProfilesState::new();
        assert!(!state.create_mode);

        state.enter_create_mode();
        assert!(state.create_mode);

        state.insert_char('t');
        state.insert_char('e');
        state.insert_char('s');
        state.insert_char('t');
        assert_eq!(state.new_name, "test");

        state.delete_char();
        assert_eq!(state.new_name, "tes");

        state.exit_create_mode();
        assert!(!state.create_mode);
        assert!(state.new_name.is_empty());
    }

    #[test]
    fn test_validate_name() {
        let mut state = ProfilesState::new();
        state.new_name = "".to_string();
        assert!(state.validate_new_name().is_err());

        state.new_name = "valid-name-123".to_string();
        assert!(state.validate_new_name().is_ok());

        state.new_name = "invalid name!".to_string();
        assert!(state.validate_new_name().is_err());
    }

    #[test]
    fn test_profile_actions_text_uses_enter_for_env() {
        assert_eq!(
            profile_actions_text(),
            "[Space] Activate | [Enter] Edit env | [n] New | [d] Delete | [Esc] Back"
        );
    }
}
