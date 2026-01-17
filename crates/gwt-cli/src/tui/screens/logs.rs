//! Logs Screen

#![allow(dead_code)] // Screen components for future use

use chrono::{DateTime, Local};
use ratatui::{prelude::*, widgets::*};
use serde_json;

/// Log entry type (local copy for TUI)
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
    pub target: String,
    pub category: Option<String>,
    pub extra: std::collections::HashMap<String, String>,
}

impl LogEntry {
    /// Format log entry for clipboard copy as JSON
    pub fn to_clipboard_string(&self) -> String {
        let mut json = serde_json::json!({
            "timestamp": self.timestamp,
            "level": self.level,
            "target": self.target,
            "message": self.message,
        });

        if let Some(ref cat) = self.category {
            json["category"] = serde_json::json!(cat);
        }

        if !self.extra.is_empty() {
            json["extra"] = serde_json::json!(self.extra);
        }

        serde_json::to_string_pretty(&json).unwrap_or_else(|_| format!("{:?}", self))
    }
}

/// Convert ISO 8601 UTC timestamp to local time display string (HH:MM:SS)
fn format_timestamp_local(timestamp: &str) -> String {
    if let Ok(utc_time) = DateTime::parse_from_rfc3339(timestamp) {
        let local_time: DateTime<Local> = utc_time.with_timezone(&Local);
        local_time.format("%H:%M:%S").to_string()
    } else {
        // Fallback: extract HH:MM:SS from string
        if let Some(t_pos) = timestamp.find('T') {
            let time_part = &timestamp[t_pos + 1..];
            if time_part.len() >= 8 {
                return time_part[..8].to_string();
            }
        }
        timestamp.to_string()
    }
}

/// Format full timestamp for detail view (YYYY-MM-DD HH:MM:SS TZ)
fn format_full_timestamp_local(timestamp: &str) -> String {
    if let Ok(utc_time) = DateTime::parse_from_rfc3339(timestamp) {
        let local_time: DateTime<Local> = utc_time.with_timezone(&Local);
        local_time.format("%Y-%m-%d %H:%M:%S %Z").to_string()
    } else {
        timestamp.to_string()
    }
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
    pub show_detail: bool,
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

    /// Toggle detail view
    pub fn toggle_detail(&mut self) {
        if self.selected_entry().is_some() {
            self.show_detail = !self.show_detail;
        }
    }

    /// Close detail view
    pub fn close_detail(&mut self) {
        self.show_detail = false;
    }

    /// Check if detail view is shown
    pub fn is_detail_shown(&self) -> bool {
        self.show_detail
    }
}

/// Render logs screen
pub fn render_logs(state: &LogsState, frame: &mut Frame, area: Rect) {
    // If detail view is shown, render it as an overlay
    if state.show_detail {
        if let Some(entry) = state.selected_entry() {
            render_detail_view(entry, frame, area);
            return;
        }
    }

    // Search bar height: 3 when searching, 0 otherwise
    let search_bar_height = if state.is_searching { 3 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),                  // Header/Filter
            Constraint::Min(0),                     // Log entries
            Constraint::Length(search_bar_height), // Search bar (only when searching)
        ])
        .split(area);

    // Header with filter info
    render_header(state, frame, chunks[0]);

    // Log entries
    render_entries(state, frame, chunks[1]);

    // Search bar (only when searching)
    if state.is_searching {
        render_search_bar(state, frame, chunks[2]);
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

    let header = Paragraph::new("").block(Block::default().borders(Borders::BOTTOM).title(title));
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
        let mut scrollbar_state = ScrollbarState::new(filtered.len()).position(state.selected);
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

fn render_log_entry(entry: &LogEntry, is_selected: bool) -> ListItem<'static> {
    let level_style = match entry.level.as_str() {
        "ERROR" => Style::default().fg(Color::Red),
        "WARN" => Style::default().fg(Color::Yellow),
        "INFO" => Style::default().fg(Color::Green),
        "DEBUG" => Style::default().fg(Color::Blue),
        "TRACE" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };

    // Convert UTC timestamp to local time
    let time_display = format_timestamp_local(&entry.timestamp);

    let spans = vec![
        Span::styled(
            format!("[{}]", time_display),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(format!("{:5}", entry.level), level_style),
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

    let search = Paragraph::new(display_text).style(text_style).block(
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


fn render_detail_view(entry: &LogEntry, frame: &mut Frame, area: Rect) {
    let level_style = match entry.level.as_str() {
        "ERROR" => Style::default().fg(Color::Red),
        "WARN" => Style::default().fg(Color::Yellow),
        "INFO" => Style::default().fg(Color::Green),
        "DEBUG" => Style::default().fg(Color::Blue),
        "TRACE" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Timestamp: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format_full_timestamp_local(&entry.timestamp)),
        ]),
        Line::from(vec![
            Span::styled("Level:     ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(&entry.level, level_style),
        ]),
        Line::from(vec![
            Span::styled("Target:    ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&entry.target),
        ]),
    ];

    // Show category if present
    if let Some(ref category) = entry.category {
        lines.push(Line::from(vec![
            Span::styled("Category:  ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(category, Style::default().fg(Color::Cyan)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Message:",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(entry.message.clone()));

    // Show extra fields if present
    if !entry.extra.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Extra Fields:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (key, value) in &entry.extra {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}: ", key), Style::default().fg(Color::DarkGray)),
                Span::raw(value),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Log Detail ")
            .title_bottom(" [c] Copy | [Esc] Close "),
    );
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
                category: Some("worktree".to_string()),
                extra: std::collections::HashMap::new(),
            },
            LogEntry {
                timestamp: "2024-01-01T12:00:01Z".to_string(),
                level: "ERROR".to_string(),
                message: "Error message".to_string(),
                target: "test".to_string(),
                category: None,
                extra: std::collections::HashMap::new(),
            },
            LogEntry {
                timestamp: "2024-01-01T12:00:02Z".to_string(),
                level: "DEBUG".to_string(),
                message: "Debug message".to_string(),
                target: "test".to_string(),
                category: Some("git".to_string()),
                extra: std::collections::HashMap::new(),
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

    #[test]
    fn test_to_clipboard_string_json_format() {
        let entry = LogEntry {
            timestamp: "2024-01-01T12:00:00Z".to_string(),
            level: "INFO".to_string(),
            message: "Test message".to_string(),
            target: "test".to_string(),
            category: Some("worktree".to_string()),
            extra: std::collections::HashMap::new(),
        };

        let clipboard_text = entry.to_clipboard_string();
        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&clipboard_text).unwrap();
        assert_eq!(parsed["timestamp"], "2024-01-01T12:00:00Z");
        assert_eq!(parsed["level"], "INFO");
        assert_eq!(parsed["target"], "test");
        assert_eq!(parsed["message"], "Test message");
        assert_eq!(parsed["category"], "worktree");
    }

    #[test]
    fn test_to_clipboard_string_without_category() {
        let entry = LogEntry {
            timestamp: "2024-01-01T12:00:00Z".to_string(),
            level: "ERROR".to_string(),
            message: "Error occurred".to_string(),
            target: "test".to_string(),
            category: None,
            extra: std::collections::HashMap::new(),
        };

        let clipboard_text = entry.to_clipboard_string();
        let parsed: serde_json::Value = serde_json::from_str(&clipboard_text).unwrap();
        assert_eq!(parsed["level"], "ERROR");
        assert!(parsed.get("category").is_none());
    }
}
