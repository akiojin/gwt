//! Logs Screen

use ratatui::{prelude::*, widgets::*};

/// Log entry type (local copy for TUI)
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
    pub target: String,
}

/// Log level filter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevelFilter {
    #[default]
    All,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevelFilter {
    fn matches(&self, level: &str) -> bool {
        match self {
            LogLevelFilter::All => true,
            LogLevelFilter::Error => level == "ERROR",
            LogLevelFilter::Warn => level == "WARN" || level == "ERROR",
            LogLevelFilter::Info => level == "INFO" || level == "WARN" || level == "ERROR",
            LogLevelFilter::Debug => level != "TRACE",
            LogLevelFilter::Trace => true,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            LogLevelFilter::All => "All",
            LogLevelFilter::Error => "Error",
            LogLevelFilter::Warn => "Warn+",
            LogLevelFilter::Info => "Info+",
            LogLevelFilter::Debug => "Debug+",
            LogLevelFilter::Trace => "Trace",
        }
    }
}

/// Logs state
#[derive(Debug, Default)]
pub struct LogsState {
    pub entries: Vec<LogEntry>,
    pub selected: usize,
    pub offset: usize,
    pub filter: LogLevelFilter,
    pub search: String,
    pub is_searching: bool,
}

impl LogsState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_entries(mut self, entries: Vec<LogEntry>) -> Self {
        self.entries = entries;
        self
    }

    /// Get filtered entries
    pub fn filtered_entries(&self) -> Vec<&LogEntry> {
        self.entries
            .iter()
            .filter(|e| self.filter.matches(&e.level))
            .filter(|e| {
                if self.search.is_empty() {
                    true
                } else {
                    let search_lower = self.search.to_lowercase();
                    e.message.to_lowercase().contains(&search_lower)
                        || e.level.to_lowercase().contains(&search_lower)
                }
            })
            .collect()
    }

    /// Select previous entry
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Select next entry
    pub fn select_next(&mut self) {
        let filtered = self.filtered_entries();
        if !filtered.is_empty() && self.selected < filtered.len() - 1 {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    /// Page up
    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
        self.ensure_visible();
    }

    /// Page down
    pub fn page_down(&mut self, page_size: usize) {
        let filtered = self.filtered_entries();
        if !filtered.is_empty() {
            self.selected = (self.selected + page_size).min(filtered.len() - 1);
            self.ensure_visible();
        }
    }

    /// Go to start
    pub fn go_home(&mut self) {
        self.selected = 0;
        self.offset = 0;
    }

    /// Go to end
    pub fn go_end(&mut self) {
        let filtered = self.filtered_entries();
        if !filtered.is_empty() {
            self.selected = filtered.len() - 1;
        }
        self.ensure_visible();
    }

    /// Cycle log level filter
    pub fn cycle_filter(&mut self) {
        self.filter = match self.filter {
            LogLevelFilter::All => LogLevelFilter::Error,
            LogLevelFilter::Error => LogLevelFilter::Warn,
            LogLevelFilter::Warn => LogLevelFilter::Info,
            LogLevelFilter::Info => LogLevelFilter::Debug,
            LogLevelFilter::Debug => LogLevelFilter::Trace,
            LogLevelFilter::Trace => LogLevelFilter::All,
        };
        self.selected = 0;
        self.offset = 0;
    }

    /// Toggle search mode
    pub fn toggle_search(&mut self) {
        self.is_searching = !self.is_searching;
        if !self.is_searching {
            self.search.clear();
            self.selected = 0;
            self.offset = 0;
        }
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        let visible_window = 10;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + visible_window {
            self.offset = self.selected.saturating_sub(visible_window - 1);
        }
    }

    /// Get selected entry
    pub fn selected_entry(&self) -> Option<&LogEntry> {
        let filtered = self.filtered_entries();
        filtered.get(self.selected).copied()
    }
}

/// Render logs screen
pub fn render_logs(
    state: &LogsState,
    frame: &mut Frame,
    area: Rect,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header/Filter
            Constraint::Min(0),    // Log entries
            Constraint::Length(3), // Instructions or search
        ])
        .split(area);

    // Header with filter info
    render_header(state, frame, chunks[0]);

    // Log entries
    render_entries(state, frame, chunks[1]);

    // Instructions or search bar
    if state.is_searching {
        render_search_bar(state, frame, chunks[2]);
    } else {
        render_instructions(frame, chunks[2]);
    }
}

fn render_header(state: &LogsState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_entries();
    let title = format!(
        " Logs ({}/{}) | Filter: {} ",
        filtered.len(),
        state.entries.len(),
        state.filter.name()
    );

    let header = Paragraph::new("")
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(header, area);
}

fn render_entries(state: &LogsState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_entries();

    if filtered.is_empty() {
        let text = if state.entries.is_empty() {
            "No log entries"
        } else {
            "No entries match filter"
        };
        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(paragraph, area);
        return;
    }

    let visible_height = area.height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .skip(state.offset)
        .take(visible_height)
        .map(|(i, entry)| render_log_entry(entry, i == state.selected))
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_widget(list, area);

    // Scrollbar
    if filtered.len() > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("^"))
            .end_symbol(Some("v"));
        let mut scrollbar_state = ScrollbarState::new(filtered.len())
            .position(state.selected);
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
}

fn render_log_entry(entry: &LogEntry, is_selected: bool) -> ListItem<'static> {
    let level_style = match entry.level.as_str() {
        "ERROR" => Style::default().fg(Color::Red),
        "WARN" => Style::default().fg(Color::Yellow),
        "INFO" => Style::default().fg(Color::Green),
        "DEBUG" => Style::default().fg(Color::Blue),
        "TRACE" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };

    // Extract time part from timestamp if possible
    let time_display = if entry.timestamp.len() >= 8 {
        // Try to extract HH:MM:SS from ISO timestamp
        if let Some(t_pos) = entry.timestamp.find('T') {
            let time_part = &entry.timestamp[t_pos + 1..];
            if time_part.len() >= 8 {
                time_part[..8].to_string()
            } else {
                entry.timestamp.clone()
            }
        } else {
            entry.timestamp.clone()
        }
    } else {
        entry.timestamp.clone()
    };

    let spans = vec![
        Span::styled(
            format!("[{}]", time_display),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:5}", entry.level),
            level_style,
        ),
        Span::raw(" "),
        Span::raw(entry.message.clone()),
    ];

    let style = if is_selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

fn render_search_bar(state: &LogsState, frame: &mut Frame, area: Rect) {
    let display_text = if state.search.is_empty() {
        "Type to search..."
    } else {
        &state.search
    };

    let text_style = if state.search.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let search = Paragraph::new(display_text)
        .style(text_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Search "),
        );
    frame.render_widget(search, area);

    // Cursor
    frame.set_cursor_position(Position::new(
        area.x + state.search.len() as u16 + 1,
        area.y + 1,
    ));
}

fn render_instructions(frame: &mut Frame, area: Rect) {
    let instructions = "[Up/Down] Navigate | [f] Filter | [/] Search | [Esc] Back";
    let paragraph = Paragraph::new(format!(" {} ", instructions))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_entries() -> Vec<LogEntry> {
        vec![
            LogEntry {
                timestamp: "2024-01-01T12:00:00Z".to_string(),
                level: "INFO".to_string(),
                message: "Test message 1".to_string(),
                target: "test".to_string(),
            },
            LogEntry {
                timestamp: "2024-01-01T12:00:01Z".to_string(),
                level: "ERROR".to_string(),
                message: "Error message".to_string(),
                target: "test".to_string(),
            },
            LogEntry {
                timestamp: "2024-01-01T12:00:02Z".to_string(),
                level: "DEBUG".to_string(),
                message: "Debug message".to_string(),
                target: "test".to_string(),
            },
        ]
    }

    #[test]
    fn test_log_filtering() {
        let state = LogsState::new().with_entries(create_test_entries());
        assert_eq!(state.filtered_entries().len(), 3);

        let mut state = state;
        state.filter = LogLevelFilter::Error;
        assert_eq!(state.filtered_entries().len(), 1);
        assert_eq!(state.filtered_entries()[0].level, "ERROR");
    }

    #[test]
    fn test_log_navigation() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_next();
        assert_eq!(state.selected, 2);

        state.select_next(); // Should not go beyond
        assert_eq!(state.selected, 2);

        state.select_prev();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn test_filter_cycle() {
        let mut state = LogsState::new();
        assert_eq!(state.filter, LogLevelFilter::All);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::Error);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::Warn);
    }
}
