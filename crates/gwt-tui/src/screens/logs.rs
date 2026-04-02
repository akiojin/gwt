//! Logs viewer screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame,
};

/// Log severity filter level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterLevel {
    #[default]
    All,
    ErrorOnly,
    WarnUp,
    InfoUp,
    DebugUp,
}

impl FilterLevel {
    /// All filter levels in display order.
    pub const ALL: [FilterLevel; 5] = [
        FilterLevel::All,
        FilterLevel::ErrorOnly,
        FilterLevel::WarnUp,
        FilterLevel::InfoUp,
        FilterLevel::DebugUp,
    ];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::ErrorOnly => "Error",
            Self::WarnUp => "Warn+",
            Self::InfoUp => "Info+",
            Self::DebugUp => "Debug+",
        }
    }

    /// Severity threshold (lower = more severe).
    fn threshold(self) -> u8 {
        match self {
            Self::All => 0,
            Self::DebugUp => 1,
            Self::InfoUp => 2,
            Self::WarnUp => 3,
            Self::ErrorOnly => 4,
        }
    }
}

/// A single log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: String,
    pub severity: String,
    pub source: String,
    pub message: String,
}

impl LogEntry {
    /// Classify severity string into (numeric_level, color).
    fn classify_severity(&self) -> (u8, Color) {
        match self.severity.to_lowercase().as_str() {
            "error" | "err" => (4, Color::Red),
            "warn" | "warning" => (3, Color::Yellow),
            "info" => (2, Color::Green),
            "debug" | "dbg" => (1, Color::DarkGray),
            _ => (0, Color::White),
        }
    }

    /// Numeric severity for filtering.
    fn severity_level(&self) -> u8 {
        self.classify_severity().0
    }

    /// Color for this severity.
    fn severity_color(&self) -> Color {
        self.classify_severity().1
    }
}

/// State for the logs screen.
#[derive(Debug, Clone, Default)]
pub struct LogsState {
    pub entries: Vec<LogEntry>,
    pub selected: usize,
    pub filter_level: FilterLevel,
    pub detail_view: bool,
}

impl LogsState {
    /// Return entries filtered by the current filter level.
    pub fn filtered_entries(&self) -> Vec<&LogEntry> {
        let threshold = self.filter_level.threshold();
        self.entries
            .iter()
            .filter(|e| {
                if threshold == 0 {
                    true
                } else {
                    e.severity_level() >= threshold
                }
            })
            .collect()
    }

    /// Get the currently selected entry from the filtered list.
    pub fn selected_entry(&self) -> Option<&LogEntry> {
        let filtered = self.filtered_entries();
        filtered.get(self.selected).copied()
    }

    /// Clamp selected index to filtered length.
    fn clamp_selected(&mut self) {
        let len = self.filtered_entries().len();
        super::clamp_index(&mut self.selected, len);
    }
}

/// Messages specific to the logs screen.
#[derive(Debug, Clone)]
pub enum LogsMessage {
    MoveUp,
    MoveDown,
    ToggleDetail,
    SetFilter(FilterLevel),
    Refresh,
    SetEntries(Vec<LogEntry>),
}

/// Update logs state in response to a message.
pub fn update(state: &mut LogsState, msg: LogsMessage) {
    match msg {
        LogsMessage::MoveUp => {
            let len = state.filtered_entries().len();
            super::move_up(&mut state.selected, len);
        }
        LogsMessage::MoveDown => {
            let len = state.filtered_entries().len();
            super::move_down(&mut state.selected, len);
        }
        LogsMessage::ToggleDetail => {
            state.detail_view = !state.detail_view;
        }
        LogsMessage::SetFilter(level) => {
            state.filter_level = level;
            state.clamp_selected();
        }
        LogsMessage::Refresh => {
            // Signal to reload logs — handled by caller
        }
        LogsMessage::SetEntries(entries) => {
            state.entries = entries;
            state.clamp_selected();
        }
    }
}

/// Render the logs screen.
pub fn render(state: &LogsState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Filter tabs
            Constraint::Min(0),   // Log list / detail
        ])
        .split(area);

    render_filter_tabs(state, frame, chunks[0]);

    if state.detail_view {
        render_detail(state, frame, chunks[1]);
    } else {
        render_log_list(state, frame, chunks[1]);
    }
}

/// Render the filter tab bar.
fn render_filter_tabs(state: &LogsState, frame: &mut Frame, area: Rect) {
    let titles: Vec<Line> = FilterLevel::ALL.iter().map(|f| Line::from(f.label())).collect();

    let active_idx = FilterLevel::ALL
        .iter()
        .position(|f| *f == state.filter_level)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Logs"))
        .select(active_idx)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, area);
}

/// Render the log entry list.
fn render_log_list(state: &LogsState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_entries();

    if filtered.is_empty() {
        let block = Block::default().borders(Borders::ALL);
        let msg = if state.entries.is_empty() {
            "No log entries"
        } else {
            "No entries match filter"
        };
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let style = super::list_item_style(idx == state.selected);

            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", entry.timestamp),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("[{:5}] ", entry.severity),
                    Style::default().fg(entry.severity_color()),
                ),
                Span::styled(
                    format!("{}: ", entry.source),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(entry.message.clone(), style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Enter: detail | r: refresh");
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Render the detail view for the selected entry.
fn render_detail(state: &LogsState, frame: &mut Frame, area: Rect) {
    let entry = state.selected_entry();
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Log Detail — Esc: back");

    match entry {
        Some(e) => {
            let text = format!(
                " Timestamp: {}\n Severity:  {}\n Source:    {}\n Message:   {}",
                e.timestamp, e.severity, e.source, e.message
            );
            let paragraph = Paragraph::new(text)
                .block(block)
                .style(Style::default().fg(Color::White));
            frame.render_widget(paragraph, area);
        }
        None => {
            let paragraph = Paragraph::new("No entry selected")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_entries() -> Vec<LogEntry> {
        vec![
            LogEntry {
                timestamp: "12:00:01".to_string(),
                severity: "ERROR".to_string(),
                source: "core".to_string(),
                message: "Failed to connect".to_string(),
            },
            LogEntry {
                timestamp: "12:00:02".to_string(),
                severity: "WARN".to_string(),
                source: "tui".to_string(),
                message: "Slow render".to_string(),
            },
            LogEntry {
                timestamp: "12:00:03".to_string(),
                severity: "INFO".to_string(),
                source: "core".to_string(),
                message: "Started session".to_string(),
            },
            LogEntry {
                timestamp: "12:00:04".to_string(),
                severity: "DEBUG".to_string(),
                source: "pty".to_string(),
                message: "Buffer flush".to_string(),
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = LogsState::default();
        assert!(state.entries.is_empty());
        assert_eq!(state.selected, 0);
        assert_eq!(state.filter_level, FilterLevel::All);
        assert!(!state.detail_view);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        update(&mut state, LogsMessage::MoveDown);
        assert_eq!(state.selected, 1);
        update(&mut state, LogsMessage::MoveDown);
        update(&mut state, LogsMessage::MoveDown);
        update(&mut state, LogsMessage::MoveDown);
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        update(&mut state, LogsMessage::MoveUp);
        assert_eq!(state.selected, 3); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = LogsState::default();
        update(&mut state, LogsMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, LogsMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn toggle_detail() {
        let mut state = LogsState::default();
        assert!(!state.detail_view);
        update(&mut state, LogsMessage::ToggleDetail);
        assert!(state.detail_view);
        update(&mut state, LogsMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn set_filter_clamps() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.selected = 3; // DEBUG entry

        update(&mut state, LogsMessage::SetFilter(FilterLevel::ErrorOnly));
        assert_eq!(state.filter_level, FilterLevel::ErrorOnly);
        assert_eq!(state.selected, 0); // clamped (only 1 error entry)
    }

    #[test]
    fn set_entries_populates() {
        let mut state = LogsState::default();
        state.selected = 99;
        update(&mut state, LogsMessage::SetEntries(sample_entries()));
        assert_eq!(state.entries.len(), 4);
        assert_eq!(state.selected, 3); // clamped
    }

    #[test]
    fn filtered_entries_error_only() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.filter_level = FilterLevel::ErrorOnly;
        let filtered = state.filtered_entries();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].severity, "ERROR");
    }

    #[test]
    fn filtered_entries_warn_up() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.filter_level = FilterLevel::WarnUp;
        let filtered = state.filtered_entries();
        assert_eq!(filtered.len(), 2); // ERROR + WARN
    }

    #[test]
    fn filtered_entries_all() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.filter_level = FilterLevel::All;
        assert_eq!(state.filtered_entries().len(), 4);
    }

    #[test]
    fn selected_entry_returns_correct() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.selected = 2;
        let entry = state.selected_entry().unwrap();
        assert_eq!(entry.severity, "INFO");
    }

    #[test]
    fn render_with_entries_does_not_panic() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Logs"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = LogsState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn render_detail_view_does_not_panic() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.detail_view = true;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
    }

    #[test]
    fn severity_colors_mapped() {
        let e = LogEntry {
            timestamp: String::new(),
            severity: "ERROR".to_string(),
            source: String::new(),
            message: String::new(),
        };
        assert_eq!(e.severity_color(), Color::Red);

        let w = LogEntry {
            timestamp: String::new(),
            severity: "WARN".to_string(),
            source: String::new(),
            message: String::new(),
        };
        assert_eq!(w.severity_color(), Color::Yellow);
    }
}
