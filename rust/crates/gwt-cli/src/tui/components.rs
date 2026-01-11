//! TUI Components

use ratatui::{
    prelude::*,
    widgets::*,
};

/// Header component with statistics
pub struct Header<'a> {
    pub title: &'a str,
    pub active_count: usize,
    pub total_count: usize,
    pub is_offline: bool,
}

impl<'a> Header<'a> {
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            active_count: 0,
            total_count: 0,
            is_offline: false,
        }
    }

    pub fn with_stats(mut self, active: usize, total: usize) -> Self {
        self.active_count = active;
        self.total_count = total;
        self
    }

    pub fn with_offline(mut self, offline: bool) -> Self {
        self.is_offline = offline;
        self
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let offline_indicator = if self.is_offline { " [OFFLINE]" } else { "" };
        let stats = if self.total_count > 0 {
            format!(" ({}/{} active)", self.active_count, self.total_count)
        } else {
            String::new()
        };

        let title = format!(" {} {}{} ", self.title, stats, offline_indicator);
        let header = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        frame.render_widget(header, area);
    }
}

/// Footer component with keybinds
pub struct Footer<'a> {
    pub keybinds: Vec<(&'a str, &'a str)>,
    pub extra_message: Option<&'a str>,
}

impl<'a> Footer<'a> {
    pub fn new(keybinds: Vec<(&'a str, &'a str)>) -> Self {
        Self {
            keybinds,
            extra_message: None,
        }
    }

    pub fn with_message(mut self, message: &'a str) -> Self {
        self.extra_message = Some(message);
        self
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let keybinds_str: Vec<String> = self
            .keybinds
            .iter()
            .map(|(key, action)| format!("[{}] {}", key, action))
            .collect();
        let mut text = keybinds_str.join(" | ");

        if let Some(msg) = self.extra_message {
            text.push_str(" | ");
            text.push_str(msg);
        }

        let footer = Paragraph::new(format!(" {} ", text))
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, area);
    }
}

/// Scrollable list with selection
pub struct ScrollableList<'a, T> {
    pub items: &'a [T],
    pub selected: usize,
    pub offset: usize,
    pub title: &'a str,
    pub render_item: fn(&T, bool) -> ListItem<'a>,
}

impl<'a, T> ScrollableList<'a, T> {
    pub fn new(items: &'a [T], title: &'a str, render_item: fn(&T, bool) -> ListItem<'a>) -> Self {
        Self {
            items,
            selected: 0,
            offset: 0,
            title,
            render_item,
        }
    }

    pub fn with_selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    pub fn visible_items(&self, visible_height: usize) -> impl Iterator<Item = (usize, &T)> {
        self.items
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(visible_height)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let inner_height = area.height.saturating_sub(2) as usize;
        let visible_items: Vec<ListItem> = self
            .visible_items(inner_height)
            .map(|(i, item)| (self.render_item)(item, i == self.selected))
            .collect();

        let list = List::new(visible_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", self.title)),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_widget(list, area);

        // Render scrollbar if needed
        if self.items.len() > inner_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("^"))
                .end_symbol(Some("v"));
            let mut scrollbar_state = ScrollbarState::new(self.items.len())
                .position(self.selected);
            frame.render_stateful_widget(
                scrollbar,
                area.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    }
}

/// Text input component
pub struct TextInput<'a> {
    pub value: &'a str,
    pub cursor: usize,
    pub label: &'a str,
    pub placeholder: &'a str,
}

impl<'a> TextInput<'a> {
    pub fn new(value: &'a str, label: &'a str) -> Self {
        Self {
            value,
            cursor: value.len(),
            label,
            placeholder: "",
        }
    }

    pub fn with_placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn with_cursor(mut self, cursor: usize) -> Self {
        self.cursor = cursor;
        self
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let display_value = if self.value.is_empty() {
            self.placeholder
        } else {
            self.value
        };

        let style = if self.value.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };

        let input = Paragraph::new(display_value)
            .style(style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", self.label)),
            );

        frame.render_widget(input, area);

        // Show cursor
        if !self.value.is_empty() || self.placeholder.is_empty() {
            frame.set_cursor_position(Position::new(
                area.x + self.cursor as u16 + 1,
                area.y + 1,
            ));
        }
    }
}

/// Dialog component
pub struct Dialog<'a> {
    pub title: &'a str,
    pub message: &'a str,
    pub buttons: Vec<&'a str>,
    pub selected_button: usize,
}

impl<'a> Dialog<'a> {
    pub fn new(title: &'a str, message: &'a str) -> Self {
        Self {
            title,
            message,
            buttons: vec!["OK"],
            selected_button: 0,
        }
    }

    pub fn with_buttons(mut self, buttons: Vec<&'a str>) -> Self {
        self.buttons = buttons;
        self
    }

    pub fn with_selected(mut self, selected: usize) -> Self {
        self.selected_button = selected;
        self
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        // Calculate dialog size
        let dialog_width = 50.min(area.width.saturating_sub(4));
        let dialog_height = 7.min(area.height.saturating_sub(4));

        let dialog_area = centered_rect(dialog_width, dialog_height, area);

        // Clear background
        frame.render_widget(Clear, dialog_area);

        // Render dialog
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // Message
                Constraint::Length(3), // Buttons
            ])
            .split(dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(format!(" {} ", self.title));

        let message = Paragraph::new(self.message)
            .alignment(Alignment::Center)
            .block(block);
        frame.render_widget(message, chunks[0]);

        // Render buttons
        let button_text: Vec<Span> = self
            .buttons
            .iter()
            .enumerate()
            .flat_map(|(i, btn)| {
                let style = if i == self.selected_button {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                vec![
                    Span::raw(" "),
                    Span::styled(format!("[{}]", btn), style),
                    Span::raw(" "),
                ]
            })
            .collect();

        let buttons = Paragraph::new(Line::from(button_text)).alignment(Alignment::Center);
        frame.render_widget(buttons, chunks[1]);
    }
}

/// Spinner component for loading states
pub struct Spinner<'a> {
    pub message: &'a str,
    pub tick: usize,
}

impl<'a> Spinner<'a> {
    const FRAMES: [&'static str; 4] = ["|", "/", "-", "\\"];

    pub fn new(message: &'a str, tick: usize) -> Self {
        Self { message, tick }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let spinner_char = Self::FRAMES[self.tick % Self::FRAMES.len()];
        let text = format!("{} {}", spinner_char, self.message);
        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(paragraph, area);
    }
}

/// Helper function to create a centered rect
pub fn centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((r.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((r.width.saturating_sub(width)) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_centered_rect() {
        let area = Rect::new(0, 0, 100, 50);
        let centered = centered_rect(20, 10, area);
        assert_eq!(centered.width, 20);
        assert_eq!(centered.height, 10);
        assert_eq!(centered.x, 40);
        assert_eq!(centered.y, 20);
    }
}
