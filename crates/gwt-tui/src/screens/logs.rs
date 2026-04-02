//! Logs viewer screen.

use gwt_notification::Severity;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame,
};

pub use gwt_notification::Notification as LogEntry;

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

    /// Minimum severity required for an entry to remain visible.
    fn minimum_severity(self) -> Option<Severity> {
        match self {
            Self::All => None,
            Self::DebugUp => Some(Severity::Debug),
            Self::InfoUp => Some(Severity::Info),
            Self::WarnUp => Some(Severity::Warn),
            Self::ErrorOnly => Some(Severity::Error),
        }
    }
}

/// State for the logs screen.
#[derive(Debug, Clone, Default)]
pub struct LogsState {
    pub(crate) entries: Vec<LogEntry>,
    pub(crate) selected: usize,
    pub(crate) filter_level: FilterLevel,
    pub(crate) detail_view: bool,
}

impl LogsState {
    /// Return entries filtered by the current filter level.
    pub fn filtered_entries(&self) -> Vec<&LogEntry> {
        match self.filter_level.minimum_severity() {
            Some(min_severity) => self
                .entries
                .iter()
                .filter(|entry| entry.severity >= min_severity)
                .collect(),
            None => self.entries.iter().collect(),
        }
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
            Constraint::Min(0),    // Log list / detail
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
    let titles: Vec<Line> = FilterLevel::ALL
        .iter()
        .map(|f| Line::from(f.label()))
        .collect();

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
                    Style::default().fg(severity_color(entry.severity)),
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
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Render the detail view for the selected entry.
fn render_detail(state: &LogsState, frame: &mut Frame, area: Rect) {
    let entry = state.selected_entry();
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Log Detail — Esc: back");

    match entry {
        Some(e) => {
            let mut text = format!(
                " Timestamp: {}\n Severity:  {}\n Source:    {}\n Message:   {}\n ID:        {}",
                e.timestamp, e.severity, e.source, e.message, e.id
            );
            if let Some(detail) = e.detail.as_deref() {
                text.push_str(&format!("\n Detail:    {}", detail));
            }
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

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Error => Color::Red,
        Severity::Warn => Color::Yellow,
        Severity::Info => Color::Green,
        Severity::Debug => Color::DarkGray,
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
            LogEntry::new(Severity::Error, "core", "Failed to connect")
                .with_detail("connection timed out"),
            LogEntry::new(Severity::Warn, "tui", "Slow render"),
            LogEntry::new(Severity::Info, "core", "Started session"),
            LogEntry::new(Severity::Debug, "pty", "Buffer flush"),
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
        assert_eq!(filtered[0].severity, Severity::Error);
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
        assert_eq!(entry.severity, Severity::Info);
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
        assert_eq!(severity_color(Severity::Error), Color::Red);
        assert_eq!(severity_color(Severity::Warn), Color::Yellow);
        assert_eq!(severity_color(Severity::Info), Color::Green);
        assert_eq!(severity_color(Severity::Debug), Color::DarkGray);
    }

    #[test]
    fn selected_entry_includes_notification_detail() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.selected = 0;
        let entry = state.selected_entry().unwrap();
        assert_eq!(entry.detail.as_deref(), Some("connection timed out"));
    }

    #[test]
    fn render_detail_includes_detail_text() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        state.detail_view = true;
        state.selected = 0;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Log Detail"));
        assert!(text.contains("connection timed out"));
    }
}
