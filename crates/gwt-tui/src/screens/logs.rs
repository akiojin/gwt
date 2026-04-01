//! Logs screen — log viewer with filtering, search, and detail view
//!
//! Migrated from gwt-cli reference with Elm Architecture adaptation.

use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

// ---------------------------------------------------------------------------
// Log entry
// ---------------------------------------------------------------------------

/// A single parsed log entry.
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
    /// Format for clipboard copy.
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

// ---------------------------------------------------------------------------
// Log level filter
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Timestamp formatting
// ---------------------------------------------------------------------------

fn format_timestamp_local(timestamp: &str) -> String {
    if let Ok(utc_time) = DateTime::parse_from_rfc3339(timestamp) {
        let local_time: DateTime<Local> = utc_time.with_timezone(&Local);
        local_time.format("%H:%M:%S").to_string()
    } else {
        if let Some(t_pos) = timestamp.find('T') {
            let time_part = &timestamp[t_pos + 1..];
            if time_part.len() >= 8 {
                return time_part[..8].to_string();
            }
        }
        timestamp.to_string()
    }
}

fn format_full_timestamp_local(timestamp: &str) -> String {
    if let Ok(utc_time) = DateTime::parse_from_rfc3339(timestamp) {
        let local_time: DateTime<Local> = utc_time.with_timezone(&Local);
        local_time.format("%Y-%m-%d %H:%M:%S %Z").to_string()
    } else {
        timestamp.to_string()
    }
}

// ---------------------------------------------------------------------------
// LogsState
// ---------------------------------------------------------------------------

/// Central state for the Logs screen.
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

    pub fn with_entries(mut self, mut entries: Vec<LogEntry>) -> Self {
        // Sort newest first
        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        self.entries = entries;
        self
    }

    /// Get filtered entries based on current level filter and search.
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

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    pub fn select_next(&mut self) {
        let filtered = self.filtered_entries();
        if !filtered.is_empty() && self.selected < filtered.len() - 1 {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
        self.ensure_visible();
    }

    pub fn page_down(&mut self, page_size: usize) {
        let filtered = self.filtered_entries();
        if !filtered.is_empty() {
            self.selected = (self.selected + page_size).min(filtered.len() - 1);
            self.ensure_visible();
        }
    }

    pub fn go_home(&mut self) {
        self.selected = 0;
        self.offset = 0;
    }

    pub fn go_end(&mut self) {
        let filtered = self.filtered_entries();
        if !filtered.is_empty() {
            self.selected = filtered.len() - 1;
        }
        self.ensure_visible();
    }

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

    pub fn toggle_search(&mut self) {
        self.is_searching = !self.is_searching;
        if !self.is_searching {
            self.search.clear();
            self.selected = 0;
            self.offset = 0;
        }
    }

    fn ensure_visible(&mut self) {
        let visible_window = 10;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + visible_window {
            self.offset = self.selected.saturating_sub(visible_window - 1);
        }
    }

    pub fn selected_entry(&self) -> Option<&LogEntry> {
        let filtered = self.filtered_entries();
        filtered.get(self.selected).copied()
    }

    pub fn toggle_detail(&mut self) {
        if self.selected_entry().is_some() {
            self.show_detail = !self.show_detail;
        }
    }

    pub fn close_detail(&mut self) {
        self.show_detail = false;
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Messages specific to the Logs screen.
#[derive(Debug)]
pub enum LogsMessage {
    Refresh,
    SelectPrev,
    SelectNext,
    PageUp,
    PageDown,
    GoHome,
    GoEnd,
    CycleFilter,
    ToggleSearch,
    ToggleDetail,
    CloseDetail,
    SearchChar(char),
    SearchBackspace,
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the logs screen into the given area.
pub fn render(state: &LogsState, buf: &mut Buffer, area: Rect) {
    // Detail view overlay
    if state.show_detail {
        if let Some(entry) = state.selected_entry() {
            render_detail_view(entry, buf, area);
            return;
        }
    }

    let search_bar_height = if state.is_searching { 3 } else { 0 };

    let chunks = Layout::vertical([
        Constraint::Length(3),                 // Header/filter
        Constraint::Min(0),                    // Log entries
        Constraint::Length(search_bar_height), // Search bar
    ])
    .split(area);

    render_header(state, buf, chunks[0]);
    render_entries(state, buf, chunks[1]);

    if state.is_searching {
        render_search_bar(state, buf, chunks[2]);
    }
}

fn render_header(state: &LogsState, buf: &mut Buffer, area: Rect) {
    let filtered = state.filtered_entries();
    let title = format!(
        " Logs ({}/{}) | Filter: {} ",
        filtered.len(),
        state.entries.len(),
        state.filter.name()
    );

    let header = Paragraph::new("").block(
        Block::default()
            .borders(Borders::BOTTOM)
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
    );
    Widget::render(header, area, buf);
}

fn render_entries(state: &LogsState, buf: &mut Buffer, area: Rect) {
    let is_empty = state.entries.is_empty();
    let filtered = state.filtered_entries();

    if filtered.is_empty() {
        let text = if is_empty {
            "No log entries. Logs are loaded from ~/.gwt/logs/"
        } else {
            "No entries match current filter"
        };
        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        Widget::render(paragraph, area, buf);
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

    let list_block = Block::default().borders(Borders::ALL);
    let list = List::new(items)
        .block(list_block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    Widget::render(list, area, buf);

    // Scrollbar
    if filtered.len() > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("^"))
            .end_symbol(Some("v"));
        let mut scrollbar_state = ScrollbarState::new(filtered.len()).position(state.selected);
        scrollbar.render(
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            buf,
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

fn render_search_bar(state: &LogsState, buf: &mut Buffer, area: Rect) {
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
    Widget::render(search, area, buf);
}

fn render_detail_view(entry: &LogEntry, buf: &mut Buffer, area: Rect) {
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
            Span::styled(entry.level.clone(), level_style),
        ]),
        Line::from(vec![
            Span::styled("Target:    ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(entry.target.clone()),
        ]),
    ];

    if let Some(ref category) = entry.category {
        lines.push(Line::from(vec![
            Span::styled("Category:  ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(category.clone(), Style::default().fg(Color::Cyan)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Message:",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(entry.message.clone()));

    if !entry.extra.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Extra Fields:",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (key, value) in &entry.extra {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}: ", key), Style::default().fg(Color::DarkGray)),
                Span::raw(value.clone()),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Log Detail ")
            .title_bottom(" [Esc] Close "),
    );
    Widget::render(paragraph, area, buf);
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Handle a key event in the logs screen.
pub fn handle_key(state: &LogsState, key: &KeyEvent) -> Option<LogsMessage> {
    // Detail view mode
    if state.show_detail {
        return match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(LogsMessage::CloseDetail),
            _ => None,
        };
    }

    // Search mode
    if state.is_searching {
        return match key.code {
            KeyCode::Esc => Some(LogsMessage::ToggleSearch),
            KeyCode::Backspace => Some(LogsMessage::SearchBackspace),
            KeyCode::Char(c) => Some(LogsMessage::SearchChar(c)),
            KeyCode::Enter => Some(LogsMessage::ToggleSearch),
            _ => None,
        };
    }

    // Normal mode
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Some(LogsMessage::SelectPrev),
        KeyCode::Down | KeyCode::Char('j') => Some(LogsMessage::SelectNext),
        KeyCode::PageUp => Some(LogsMessage::PageUp),
        KeyCode::PageDown => Some(LogsMessage::PageDown),
        KeyCode::Home | KeyCode::Char('g') => Some(LogsMessage::GoHome),
        KeyCode::End | KeyCode::Char('G') => Some(LogsMessage::GoEnd),
        KeyCode::Char('f') => Some(LogsMessage::CycleFilter),
        KeyCode::Char('/') => Some(LogsMessage::ToggleSearch),
        KeyCode::Enter => Some(LogsMessage::ToggleDetail),
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(LogsMessage::Refresh)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Log file loading
// ---------------------------------------------------------------------------

/// Load log entries from `~/.gwt/logs/` directory.
pub fn load_log_entries() -> Vec<LogEntry> {
    let log_dir = dirs::home_dir()
        .map(|h| h.join(".gwt").join("logs"))
        .unwrap_or_default();

    if !log_dir.is_dir() {
        return Vec::new();
    }

    let mut entries = Vec::new();

    // Recursively scan subdirectories for log files
    // Log files are stored as ~/.gwt/logs/{branch}/gwt.jsonl.{date}
    fn scan_dir(dir: &std::path::Path, entries: &mut Vec<LogEntry>, depth: usize) {
        if depth > 3 {
            return;
        }
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_dir(&path, entries, depth + 1);
            } else if path.is_file() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                // Match: *.log, *.json, *.jsonl, gwt.jsonl.* (date-suffixed)
                if name.ends_with(".log")
                    || name.ends_with(".json")
                    || name.ends_with(".jsonl")
                    || name.contains(".jsonl.")
                {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        for line in content.lines().rev().take(200) {
                            if let Some(entry) = parse_json_log_line(line) {
                                entries.push(entry);
                            }
                        }
                    }
                }
            }
        }
    }

    scan_dir(&log_dir, &mut entries, 0);

    // Sort newest first
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries
}

/// Parse a single JSON log line (tracing-subscriber JSON format).
fn parse_json_log_line(line: &str) -> Option<LogEntry> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;

    let timestamp = obj
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let level = obj
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("INFO")
        .to_string();
    let message = obj
        .get("fields")
        .and_then(|f| f.get("message"))
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("message").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let target = obj
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let category = obj
        .get("fields")
        .and_then(|f| f.get("category"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Collect extra fields
    let mut extra = std::collections::HashMap::new();
    if let Some(fields) = obj.get("fields").and_then(|f| f.as_object()) {
        for (k, v) in fields {
            if k != "message" && k != "category" {
                extra.insert(k.clone(), v.to_string());
            }
        }
    }

    Some(LogEntry {
        timestamp,
        level,
        message,
        target,
        category,
        extra,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    fn test_log_filtering_all() {
        let state = LogsState::new().with_entries(create_test_entries());
        assert_eq!(state.filtered_entries().len(), 3);
    }

    #[test]
    fn test_log_filtering_error() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.filter = LogLevelFilter::Error;
        assert_eq!(state.filtered_entries().len(), 1);
        assert_eq!(state.filtered_entries()[0].level, "ERROR");
    }

    #[test]
    fn test_log_filtering_warn_includes_error() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.filter = LogLevelFilter::Warn;
        assert_eq!(state.filtered_entries().len(), 1); // only ERROR, no WARN entries
    }

    #[test]
    fn test_log_navigation() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_next();
        assert_eq!(state.selected, 2);

        state.select_next();
        assert_eq!(state.selected, 2); // clamped

        state.select_prev();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn test_page_navigation() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.page_down(10);
        assert_eq!(state.selected, 2); // clamped to end

        state.page_up(10);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_home_end() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.go_end();
        assert_eq!(state.selected, 2);

        state.go_home();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_filter_cycle() {
        let mut state = LogsState::new();
        assert_eq!(state.filter, LogLevelFilter::All);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::Error);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::Warn);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::Info);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::Debug);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::Trace);

        state.cycle_filter();
        assert_eq!(state.filter, LogLevelFilter::All);
    }

    #[test]
    fn test_search_filter() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.search = "Error".to_string();
        let filtered = state.filtered_entries();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].level, "ERROR");
    }

    #[test]
    fn test_toggle_search() {
        let mut state = LogsState::new();
        assert!(!state.is_searching);

        state.toggle_search();
        assert!(state.is_searching);

        state.search = "test".to_string();
        state.toggle_search();
        assert!(!state.is_searching);
        assert!(state.search.is_empty());
    }

    #[test]
    fn test_detail_toggle() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        assert!(!state.show_detail);

        state.toggle_detail();
        assert!(state.show_detail);

        state.close_detail();
        assert!(!state.show_detail);
    }

    #[test]
    fn test_selected_entry() {
        let state = LogsState::new().with_entries(create_test_entries());
        assert!(state.selected_entry().is_some());

        let empty = LogsState::new();
        assert!(empty.selected_entry().is_none());
    }

    #[test]
    fn test_to_clipboard_string() {
        let entry = LogEntry {
            timestamp: "2024-01-01T12:00:00Z".to_string(),
            level: "INFO".to_string(),
            message: "Test message".to_string(),
            target: "test".to_string(),
            category: Some("worktree".to_string()),
            extra: std::collections::HashMap::new(),
        };

        let json = entry.to_clipboard_string();
        assert!(json.contains("Test message"));
        assert!(json.contains("worktree"));
    }

    #[test]
    fn test_parse_json_log_line() {
        let line = r#"{"timestamp":"2024-01-01T12:00:00Z","level":"INFO","target":"test","fields":{"message":"hello","category":"git"}}"#;
        let entry = parse_json_log_line(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.message, "hello");
        assert_eq!(entry.category, Some("git".to_string()));
    }

    #[test]
    fn test_parse_json_log_line_invalid() {
        assert!(parse_json_log_line("not json").is_none());
        assert!(parse_json_log_line("").is_none());
    }

    #[test]
    fn test_entries_sorted_newest_first() {
        let state = LogsState::new().with_entries(create_test_entries());
        assert_eq!(state.entries[0].timestamp, "2024-01-01T12:00:02Z");
        assert_eq!(state.entries[2].timestamp, "2024-01-01T12:00:00Z");
    }

    #[test]
    fn render_smoke_test() {
        let state = LogsState::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_with_entries_smoke_test() {
        let state = LogsState::new().with_entries(create_test_entries());
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_detail_view_smoke_test() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.show_detail = true;
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_search_smoke_test() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.is_searching = true;
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&state, &mut buf, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn handle_key_navigation() {
        let state = LogsState::new();
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key),
            Some(LogsMessage::SelectNext)
        ));

        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key),
            Some(LogsMessage::SelectPrev)
        ));
    }

    #[test]
    fn handle_key_filter() {
        let state = LogsState::new();
        let key = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key),
            Some(LogsMessage::CycleFilter)
        ));
    }

    #[test]
    fn handle_key_search() {
        let state = LogsState::new();
        let key = KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key),
            Some(LogsMessage::ToggleSearch)
        ));
    }

    #[test]
    fn handle_key_detail() {
        let state = LogsState::new();
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key),
            Some(LogsMessage::ToggleDetail)
        ));
    }

    #[test]
    fn handle_key_in_detail_mode() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.show_detail = true;
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key),
            Some(LogsMessage::CloseDetail)
        ));
    }

    #[test]
    fn handle_key_in_search_mode() {
        let mut state = LogsState::new();
        state.is_searching = true;
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key),
            Some(LogsMessage::SearchChar('a'))
        ));
    }

    #[test]
    fn logs_message_is_debug() {
        let msg = LogsMessage::Refresh;
        assert!(format!("{msg:?}").contains("Refresh"));
    }

    #[test]
    fn format_timestamp_local_parses_rfc3339() {
        let result = format_timestamp_local("2024-01-15T10:30:00Z");
        assert!(!result.is_empty());
        // The result will be in local time, just check it's not the raw input
        assert!(!result.contains("T"));
    }

    #[test]
    fn format_timestamp_local_fallback() {
        let result = format_timestamp_local("not a timestamp");
        assert_eq!(result, "not a timestamp");
    }

    #[test]
    fn format_full_timestamp_parses_rfc3339() {
        let result = format_full_timestamp_local("2024-01-15T10:30:00Z");
        assert!(!result.is_empty());
        assert!(result.contains("2024"));
    }
}
