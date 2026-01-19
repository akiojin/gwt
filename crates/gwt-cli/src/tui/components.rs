//! TUI Components

#![allow(dead_code)] // UI components for future use

use ratatui::{prelude::*, widgets::*};

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

        let footer =
            Paragraph::new(format!(" {} ", text)).block(Block::default().borders(Borders::ALL));
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
            let mut scrollbar_state = ScrollbarState::new(self.items.len()).position(self.selected);
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

        let input = Paragraph::new(display_value).style(style).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", self.label)),
        );

        frame.render_widget(input, area);

        // Show cursor
        if !self.value.is_empty() || self.placeholder.is_empty() {
            frame.set_cursor_position(Position::new(area.x + self.cursor as u16 + 1, area.y + 1));
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

/// Summary panel component for displaying branch details
pub struct SummaryPanel<'a> {
    /// Branch summary data
    pub summary: &'a gwt_core::git::BranchSummary,
    /// Whether AI settings are enabled
    pub ai_enabled: bool,
    /// Animation tick for spinner
    pub tick: usize,
}

impl<'a> SummaryPanel<'a> {
    const SPINNER_FRAMES: [&'static str; 4] = ["|", "/", "-", "\\"];
    const PANEL_HEIGHT: u16 = 12;

    pub fn new(summary: &'a gwt_core::git::BranchSummary) -> Self {
        Self {
            summary,
            ai_enabled: false,
            tick: 0,
        }
    }

    pub fn with_ai_enabled(mut self, enabled: bool) -> Self {
        self.ai_enabled = enabled;
        self
    }

    pub fn with_tick(mut self, tick: usize) -> Self {
        self.tick = tick;
        self
    }

    /// Get the required height for this panel
    pub fn height() -> u16 {
        Self::PANEL_HEIGHT
    }

    /// Render the summary panel
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let title = format!(" [{}] Details ", self.summary.branch_name);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        // Calculate section layout
        let sections = self.build_sections();
        let constraints: Vec<Constraint> = sections
            .iter()
            .map(|(_, lines)| Constraint::Length(*lines as u16))
            .collect();

        if constraints.is_empty() {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner_area);

        for (i, (content, _)) in sections.iter().enumerate() {
            if i < chunks.len() {
                let paragraph = Paragraph::new(content.clone());
                frame.render_widget(paragraph, chunks[i]);
            }
        }
    }

    /// Build sections content with their line counts
    fn build_sections(&self) -> Vec<(Vec<Line<'static>>, usize)> {
        let mut sections = Vec::new();

        // Commits section
        sections.push(self.build_commits_section());

        // Stats section (only if worktree exists)
        if self.summary.has_worktree() {
            sections.push(self.build_stats_section());
        }

        // Meta section
        sections.push(self.build_meta_section());

        // AI Summary section (only if enabled and available)
        if self.ai_enabled {
            if let Some(ai_section) = self.build_ai_section() {
                sections.push(ai_section);
            }
        }

        sections
    }

    fn build_commits_section(&self) -> (Vec<Line<'static>>, usize) {
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Commits:",
            Style::default().fg(Color::Yellow),
        )));

        if self.summary.loading.commits {
            let spinner = Self::SPINNER_FRAMES[self.tick % Self::SPINNER_FRAMES.len()];
            lines.push(Line::from(format!("  {} Loading...", spinner)));
        } else if let Some(err) = &self.summary.errors.commits {
            lines.push(Line::from(Span::styled(
                format!("  (Failed to load: {})", err),
                Style::default().fg(Color::Red),
            )));
        } else if self.summary.commits.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No commits yet",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            // T204: Truncate long commit messages with "..."
            const MAX_MSG_LEN: usize = 50;
            for commit in self.summary.commits.iter().take(5) {
                let hash_span = Span::styled(
                    commit.hash.chars().take(7).collect::<String>(),
                    Style::default().fg(Color::Cyan),
                );
                let truncated_msg = if commit.message.len() > MAX_MSG_LEN {
                    format!("{}...", &commit.message[..MAX_MSG_LEN - 3])
                } else {
                    commit.message.clone()
                };
                let msg_span = Span::raw(format!(" {}", truncated_msg));
                lines.push(Line::from(vec![Span::raw("  "), hash_span, msg_span]));
            }
        }

        let count = lines.len();
        (lines, count)
    }

    fn build_stats_section(&self) -> (Vec<Line<'static>>, usize) {
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Stats:",
            Style::default().fg(Color::Yellow),
        )));

        if self.summary.loading.stats {
            let spinner = Self::SPINNER_FRAMES[self.tick % Self::SPINNER_FRAMES.len()];
            lines.push(Line::from(format!("  {} Loading...", spinner)));
        } else if let Some(err) = &self.summary.errors.stats {
            lines.push(Line::from(Span::styled(
                format!("  (Failed to load: {})", err),
                Style::default().fg(Color::Red),
            )));
        } else if let Some(stats) = &self.summary.stats {
            let mut parts = Vec::new();

            // Files and lines
            if stats.files_changed > 0 {
                parts.push(format!(
                    "{} file{}, +{}/-{} lines",
                    stats.files_changed,
                    if stats.files_changed == 1 { "" } else { "s" },
                    stats.insertions,
                    stats.deletions
                ));
            }

            // Status flags
            if stats.has_uncommitted {
                parts.push("Uncommitted changes".to_string());
            }
            if stats.has_unpushed {
                parts.push("Unpushed commits".to_string());
            }

            if parts.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No changes",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                lines.push(Line::from(format!("  {}", parts.join(" | "))));
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  No data",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let count = lines.len();
        (lines, count)
    }

    fn build_meta_section(&self) -> (Vec<Line<'static>>, usize) {
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Meta:",
            Style::default().fg(Color::Yellow),
        )));

        if self.summary.loading.meta {
            let spinner = Self::SPINNER_FRAMES[self.tick % Self::SPINNER_FRAMES.len()];
            lines.push(Line::from(format!("  {} Loading...", spinner)));
        } else if let Some(err) = &self.summary.errors.meta {
            lines.push(Line::from(Span::styled(
                format!("  (Failed to load: {})", err),
                Style::default().fg(Color::Red),
            )));
        } else if let Some(meta) = &self.summary.meta {
            let mut parts = Vec::new();

            // Ahead/behind (only if upstream exists)
            if let Some(upstream) = &meta.upstream {
                if meta.ahead > 0 || meta.behind > 0 {
                    parts.push(format!(
                        "+{} -{} from {}",
                        meta.ahead, meta.behind, upstream
                    ));
                } else {
                    parts.push(format!("Up to date with {}", upstream));
                }
            }

            // Last commit time
            if let Some(relative) = meta.relative_time() {
                parts.push(format!("Last commit: {}", relative));
            }

            if parts.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No upstream",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                for part in parts {
                    lines.push(Line::from(format!("  {}", part)));
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  No data",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let count = lines.len();
        (lines, count)
    }

    fn build_ai_section(&self) -> Option<(Vec<Line<'static>>, usize)> {
        // Don't show section if AI had an error
        if self.summary.errors.ai_summary.is_some() {
            return None;
        }

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Summary:",
            Style::default().fg(Color::Yellow),
        )));

        if self.summary.loading.ai_summary {
            let spinner = Self::SPINNER_FRAMES[self.tick % Self::SPINNER_FRAMES.len()];
            lines.push(Line::from(format!("  {} Generating...", spinner)));
        } else if let Some(summary) = &self.summary.ai_summary {
            for line in summary.iter().take(3) {
                lines.push(Line::from(format!("  {}", line)));
            }
        } else {
            // No summary available yet
            return None;
        }

        let count = lines.len();
        Some((lines, count))
    }

    /// Truncate a string to fit within a given width, adding "..." if truncated
    #[allow(dead_code)]
    pub fn truncate_string(s: &str, max_width: usize) -> String {
        if s.len() <= max_width {
            s.to_string()
        } else if max_width <= 3 {
            ".".repeat(max_width)
        } else {
            format!("{}...", &s[..max_width - 3])
        }
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
