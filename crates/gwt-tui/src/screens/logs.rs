//! Logs viewer screen.

use gwt_notification::Severity;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::theme;

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

    /// Cycle to the next filter level.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::ErrorOnly,
            Self::ErrorOnly => Self::WarnUp,
            Self::WarnUp => Self::InfoUp,
            Self::InfoUp => Self::DebugUp,
            Self::DebugUp => Self::All,
        }
    }

    /// Cycle to the previous filter level.
    pub fn prev(self) -> Self {
        match self {
            Self::All => Self::DebugUp,
            Self::ErrorOnly => Self::All,
            Self::WarnUp => Self::ErrorOnly,
            Self::InfoUp => Self::WarnUp,
            Self::DebugUp => Self::InfoUp,
        }
    }
}

/// State for the logs screen.
#[derive(Debug, Clone)]
pub struct LogsState {
    pub(crate) entries: Vec<LogEntry>,
    pub(crate) selected: usize,
    pub(crate) filter_level: FilterLevel,
    pub(crate) detail_view: bool,
    pub(crate) show_debug: bool,
}

impl Default for LogsState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            filter_level: FilterLevel::default(),
            detail_view: false,
            show_debug: true,
        }
    }
}

impl LogsState {
    /// Return entries filtered by the current filter level.
    pub fn filtered_entries(&self) -> Vec<&LogEntry> {
        match self.filter_level.minimum_severity() {
            Some(min_severity) => self
                .entries
                .iter()
                .filter(|entry| entry.severity >= min_severity)
                .filter(|entry| self.show_debug || entry.severity != Severity::Debug)
                .collect(),
            None => self
                .entries
                .iter()
                .filter(|entry| self.show_debug || entry.severity != Severity::Debug)
                .collect(),
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
    CycleFilter,
    ToggleDebugVisibility,
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
        LogsMessage::CycleFilter => {
            state.filter_level = state.filter_level.next();
            state.clamp_selected();
        }
        LogsMessage::ToggleDebugVisibility => {
            state.show_debug = !state.show_debug;
            state.clamp_selected();
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
/// Render the logs screen (borderless — outer pane border is handled by app.rs).
pub fn render(state: &LogsState, frame: &mut Frame, area: Rect) {
    // Filter sub-tab header line
    let active_idx = FilterLevel::ALL
        .iter()
        .position(|f| *f == state.filter_level)
        .unwrap_or(0);
    let labels: Vec<&str> = FilterLevel::ALL.iter().map(|f| f.label()).collect();
    let mut tab_title = super::build_tab_title(&labels, active_idx);
    // Append debug visibility indicator
    tab_title.spans.push(Span::raw(format!(
        " {} Debug: {}",
        theme::icon::SEPARATOR_VERT,
        if state.show_debug { "on" } else { "off" }
    )));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new(tab_title);
    frame.render_widget(header, chunks[0]);

    if state.detail_view {
        render_detail(state, frame, chunks[1]);
    } else {
        render_log_list(state, frame, chunks[1]);
    }
}

/// Render the log entry list.
fn render_log_list(state: &LogsState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_entries();

    if filtered.is_empty() {
        let block = Block::default();
        let msg = if state.entries.is_empty() {
            "No log entries"
        } else {
            "No entries match filter"
        };
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let style = super::list_item_style(idx == state.selected);
            let severity = format!("[{:5}]", entry.severity);

            let line = Line::from(vec![
                Span::styled(
                    format!("{:<32}", entry.timestamp),
                    theme::style::muted_text(),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<8}", severity),
                    Style::default().fg(severity_color(entry.severity)),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<12}", entry.source),
                    Style::default().fg(theme::color::FOCUS),
                ),
                Span::raw(" "),
                Span::styled(entry.message.clone(), style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().title(" Enter: detail | r: refresh");
    let list = List::new(items).block(block).highlight_style(theme::style::active_item());
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Render the detail view for the selected entry.
fn render_detail(state: &LogsState, frame: &mut Frame, area: Rect) {
    let entry = state.selected_entry();
    let block = Block::default().title("Log Detail — Esc: back");

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
                .style(Style::default().fg(theme::color::TEXT_PRIMARY));
            frame.render_widget(paragraph, area);
        }
        None => {
            let paragraph = Paragraph::new("No entry selected")
                .block(block)
                .style(theme::style::muted_text());
            frame.render_widget(paragraph, area);
        }
    }
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Error => theme::color::ERROR,
        Severity::Warn => theme::color::ACTIVE,
        Severity::Info => theme::color::SUCCESS,
        Severity::Debug => theme::color::SURFACE,
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
        assert!(state.show_debug);
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
    fn filter_level_next_cycles_through_all_levels() {
        assert_eq!(FilterLevel::All.next(), FilterLevel::ErrorOnly);
        assert_eq!(FilterLevel::ErrorOnly.next(), FilterLevel::WarnUp);
        assert_eq!(FilterLevel::WarnUp.next(), FilterLevel::InfoUp);
        assert_eq!(FilterLevel::InfoUp.next(), FilterLevel::DebugUp);
        assert_eq!(FilterLevel::DebugUp.next(), FilterLevel::All);
    }

    #[test]
    fn filter_level_prev_cycles_through_all_levels() {
        assert_eq!(FilterLevel::All.prev(), FilterLevel::DebugUp);
        assert_eq!(FilterLevel::DebugUp.prev(), FilterLevel::InfoUp);
        assert_eq!(FilterLevel::InfoUp.prev(), FilterLevel::WarnUp);
        assert_eq!(FilterLevel::WarnUp.prev(), FilterLevel::ErrorOnly);
        assert_eq!(FilterLevel::ErrorOnly.prev(), FilterLevel::All);
    }

    #[test]
    fn cycle_filter_advances_through_levels() {
        let mut state = LogsState::default();
        update(&mut state, LogsMessage::CycleFilter);
        assert_eq!(state.filter_level, FilterLevel::ErrorOnly);
        update(&mut state, LogsMessage::CycleFilter);
        assert_eq!(state.filter_level, FilterLevel::WarnUp);
        update(&mut state, LogsMessage::CycleFilter);
        assert_eq!(state.filter_level, FilterLevel::InfoUp);
        update(&mut state, LogsMessage::CycleFilter);
        assert_eq!(state.filter_level, FilterLevel::DebugUp);
        update(&mut state, LogsMessage::CycleFilter);
        assert_eq!(state.filter_level, FilterLevel::All);
    }

    #[test]
    fn toggle_debug_visibility_hides_and_restores_debug_entries() {
        let mut state = LogsState::default();
        state.entries = sample_entries();
        assert_eq!(state.filtered_entries().len(), 4);

        update(&mut state, LogsMessage::ToggleDebugVisibility);
        assert_eq!(state.filtered_entries().len(), 3);
        assert!(state
            .filtered_entries()
            .iter()
            .all(|entry| entry.severity != Severity::Debug));

        update(&mut state, LogsMessage::ToggleDebugVisibility);
        assert_eq!(state.filtered_entries().len(), 4);
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
        assert!(text.contains("Debug: on"));
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

    #[test]
    fn render_log_list_uses_stable_columns() {
        let mut state = LogsState::default();
        let entries = sample_entries();
        let expected_timestamp = format!("{}", entries[1].timestamp);
        state.entries = entries;
        state.filter_level = FilterLevel::WarnUp;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let mut found = None;
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if row.contains("Slow render") {
                found = Some(row);
                break;
            }
        }

        let row = found.expect("rendered log row");
        let row = row.trim_start_matches('│').trim_start();
        assert!(row.contains(&expected_timestamp), "{row:?}");
        assert!(row.contains("[WARN]"));
        assert!(row.contains("tui"));
        assert!(row.contains("Slow render"));
        let time_pos = row.find(&expected_timestamp).unwrap();
        let severity_pos = row.find("[WARN]").unwrap();
        let source_pos = row.find("tui").unwrap();
        let message_pos = row.find("Slow render").unwrap();
        assert!(time_pos < severity_pos);
        assert!(severity_pos < source_pos);
        assert!(source_pos < message_pos);
    }

    #[test]
    fn render_filter_tabs_show_active_warn_filter() {
        let mut state = LogsState::default();
        state.filter_level = FilterLevel::WarnUp;
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
        assert!(text.contains("Warn+"));
        assert!(text.contains("Debug: on"));
    }

    #[test]
    fn render_filter_title_reflects_debug_visibility() {
        let mut state = LogsState::default();
        state.show_debug = false;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let row: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(row.contains("Debug: off"));
    }
}
