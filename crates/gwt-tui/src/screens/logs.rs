//! Logs screen — log viewer with filtering, search, and detail view
//!
//! Migrated from gwt-cli reference with Elm Architecture adaptation.

use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::BufRead;
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::collections::BTreeMap;
use std::path::Path;

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
    pub event: Option<String>,
    pub result: Option<String>,
    pub workspace: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub extra: BTreeMap<String, String>,
}

impl LogEntry {
    /// Parse a log entry from a JSON line (tracing-subscriber JSON format).
    pub fn from_json_line(line: &str, workspace_hint: Option<&str>) -> Option<Self> {
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
            .map(ToString::to_string);
        let event = obj
            .get("fields")
            .and_then(|f| f.get("event"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let result = obj
            .get("fields")
            .and_then(|f| f.get("result"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let workspace = obj
            .get("fields")
            .and_then(|f| f.get("workspace"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| workspace_hint.map(ToString::to_string));
        let error_code = obj
            .get("fields")
            .and_then(|f| f.get("error_code"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let error_detail = obj
            .get("fields")
            .and_then(|f| f.get("error_detail"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        Some(Self {
            timestamp,
            level,
            message,
            target,
            category,
            event,
            result,
            workspace,
            error_code,
            error_detail,
            extra: BTreeMap::new(),
        })
    }

    pub fn searchable_text(&self) -> String {
        let mut fields = vec![
            self.timestamp.as_str(),
            self.level.as_str(),
            self.message.as_str(),
            self.target.as_str(),
        ];

        if let Some(category) = &self.category {
            fields.push(category);
        }
        if let Some(event) = &self.event {
            fields.push(event);
        }
        if let Some(result) = &self.result {
            fields.push(result);
        }
        if let Some(workspace) = &self.workspace {
            fields.push(workspace);
        }
        if let Some(error_code) = &self.error_code {
            fields.push(error_code);
        }
        if let Some(error_detail) = &self.error_detail {
            fields.push(error_detail);
        }

        let mut out = fields.join("\n");
        for (key, value) in &self.extra {
            out.push('\n');
            out.push_str(key);
            out.push('=');
            out.push_str(value);
        }
        out
    }

    fn summary_context(&self) -> String {
        let mut parts = Vec::new();
        if let Some(category) = &self.category {
            if let Some(event) = &self.event {
                parts.push(format!("{category}/{event}"));
            } else {
                parts.push(category.clone());
            }
        } else if let Some(event) = &self.event {
            parts.push(event.clone());
        }
        if let Some(result) = &self.result {
            parts.push(result.clone());
        }
        if let Some(error_code) = &self.error_code {
            parts.push(error_code.clone());
        }
        parts.join(" ")
    }

    fn summary_message(&self) -> String {
        if !self.message.trim().is_empty() {
            self.message.clone()
        } else if let Some(event) = &self.event {
            event.replace('_', " ")
        } else {
            self.target.clone()
        }
    }

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
        if let Some(ref event) = self.event {
            json["event"] = serde_json::json!(event);
        }
        if let Some(ref result) = self.result {
            json["result"] = serde_json::json!(result);
        }
        if let Some(ref workspace) = self.workspace {
            json["workspace"] = serde_json::json!(workspace);
        }
        if let Some(ref error_code) = self.error_code {
            json["error_code"] = serde_json::json!(error_code);
        }
        if let Some(ref error_detail) = self.error_detail {
            json["error_detail"] = serde_json::json!(error_detail);
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
                    e.searchable_text().to_lowercase().contains(&search_lower)
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

    let context = entry.summary_context();
    let message = entry.summary_message();

    let mut spans = vec![
        Span::styled(
            format!("[{}]", time_display),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(format!("{:5}", entry.level), level_style),
    ];
    if !context.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(context, Style::default().fg(Color::Cyan)));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::raw(message));

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
    if let Some(ref event) = entry.event {
        lines.push(Line::from(vec![
            Span::styled("Event:     ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(event.clone()),
        ]));
    }
    if let Some(ref result) = entry.result {
        lines.push(Line::from(vec![
            Span::styled("Result:    ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(result.clone()),
        ]));
    }
    if let Some(ref workspace) = entry.workspace {
        lines.push(Line::from(vec![
            Span::styled("Workspace: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(workspace.clone()),
        ]));
    }
    if let Some(ref error_code) = entry.error_code {
        lines.push(Line::from(vec![
            Span::styled("ErrorCode: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(error_code.clone(), Style::default().fg(Color::Red)),
        ]));
    }
    if let Some(ref error_detail) = entry.error_detail {
        lines.push(Line::from(vec![
            Span::styled("Error:     ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(error_detail.clone()),
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

const MAX_LOG_ENTRIES: usize = 1000;
const MAX_LOG_FILES: usize = 14;
const MAX_LOG_FILE_LINES: usize = 10_000;

/// Load log entries from the current workspace's `~/.gwt/logs/{workspace}/` directory.
pub fn load_log_entries(repo_root: &Path) -> Vec<LogEntry> {
    let Some(log_root) = dirs::home_dir().map(|h| h.join(".gwt").join("logs")) else {
        return Vec::new();
    };
    let workspace_name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string);
    load_log_entries_from_root(&log_root, workspace_name.as_deref())
}

fn load_log_entries_from_root(log_root: &Path, workspace_name: Option<&str>) -> Vec<LogEntry> {
    if !log_root.is_dir() {
        return Vec::new();
    }

    let mut entries = Vec::new();

    if let Some(workspace_name) = workspace_name {
        let workspace_dir = log_root.join(workspace_name);
        entries.extend(load_entries_from_dir(&workspace_dir, Some(workspace_name)));
    }

    if entries.is_empty() {
        let read_dir = match std::fs::read_dir(log_root) {
            Ok(read_dir) => read_dir,
            Err(_) => return entries,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let hint = path.file_name().and_then(|name| name.to_str());
                entries.extend(load_entries_from_dir(&path, hint));
            }
        }
    }

    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries.truncate(MAX_LOG_ENTRIES);
    entries
}

fn load_entries_from_dir(log_dir: &Path, workspace_hint: Option<&str>) -> Vec<LogEntry> {
    if !log_dir.is_dir() {
        return Vec::new();
    }

    let mut log_files: Vec<_> = match std::fs::read_dir(log_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "log" || ext == "json" || ext == "jsonl")
            })
            .map(|e| e.path())
            .collect(),
        Err(_) => return Vec::new(),
    };
    log_files.sort();
    log_files.reverse();

    let mut entries = Vec::new();
    for file in log_files.into_iter().take(MAX_LOG_FILES) {
        let Ok(reader) = std::fs::File::open(&file) else {
            continue;
        };
        let reader = std::io::BufReader::new(reader);
        let mut line_count = 0;
        for line in reader.lines() {
            if line_count >= MAX_LOG_FILE_LINES {
                break;
            }
            let Ok(line) = line else { continue };
            if let Some(entry) = LogEntry::from_json_line(&line, workspace_hint) {
                entries.push(entry);
            }
            line_count += 1;
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_entries() -> Vec<LogEntry> {
        vec![
            LogEntry {
                timestamp: "2024-01-01T12:00:00Z".to_string(),
                level: "INFO".to_string(),
                message: "Test message 1".to_string(),
                target: "test".to_string(),
                category: Some("worktree".to_string()),
                event: Some("refresh".to_string()),
                result: Some("success".to_string()),
                workspace: Some("feature-1776".to_string()),
                error_code: None,
                error_detail: None,
                extra: BTreeMap::new(),
            },
            LogEntry {
                timestamp: "2024-01-01T12:00:01Z".to_string(),
                level: "ERROR".to_string(),
                message: "Error message".to_string(),
                target: "test".to_string(),
                category: Some("ui".to_string()),
                event: Some("open_spec_detail".to_string()),
                result: Some("failure".to_string()),
                workspace: Some("feature-1776".to_string()),
                error_code: Some("SPEC_READ_FAILED".to_string()),
                error_detail: Some("missing spec.md".to_string()),
                extra: BTreeMap::new(),
            },
            LogEntry {
                timestamp: "2024-01-01T12:00:02Z".to_string(),
                level: "DEBUG".to_string(),
                message: "Debug message".to_string(),
                target: "test".to_string(),
                category: Some("git".to_string()),
                event: Some("refresh_logs".to_string()),
                result: Some("start".to_string()),
                workspace: Some("feature-1776".to_string()),
                error_code: None,
                error_detail: None,
                extra: BTreeMap::new(),
            },
        ]
    }

    fn parse_core_log(line: &str, workspace_hint: Option<&str>) -> LogEntry {
        LogEntry::from_json_line(line, workspace_hint).expect("failed to parse log line")
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
            event: Some("refresh".to_string()),
            result: Some("success".to_string()),
            workspace: Some("feature-1776".to_string()),
            error_code: None,
            error_detail: None,
            extra: BTreeMap::new(),
        };

        let json = entry.to_clipboard_string();
        assert!(json.contains("Test message"));
        assert!(json.contains("worktree"));
        assert!(json.contains("feature-1776"));
    }

    #[test]
    fn test_from_core_entry_preserves_structured_fields() {
        let line = r#"{"timestamp":"2024-01-01T12:00:00Z","level":"INFO","target":"test","fields":{"message":"hello","category":"git"}}"#;
        let entry = parse_core_log(line, Some("feature-1776"));
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.message, "hello");
        assert_eq!(entry.category, Some("git".to_string()));
        assert_eq!(entry.workspace, Some("feature-1776".to_string()));
    }

    #[test]
    fn test_search_filter_matches_event_and_error_code() {
        let mut state = LogsState::new().with_entries(create_test_entries());
        state.search = "SPEC_READ_FAILED".to_string();
        assert_eq!(state.filtered_entries().len(), 1);
        state.search = "open_spec_detail".to_string();
        assert_eq!(state.filtered_entries().len(), 1);
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

    #[test]
    fn load_log_entries_from_root_prefers_current_workspace_dir() {
        let temp = TempDir::new().unwrap();
        let feature_dir = temp.path().join("feature-1776");
        let other_dir = temp.path().join("other-workspace");
        fs::create_dir_all(&feature_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();

        fs::write(
            feature_dir.join("gwt.jsonl.2026-04-01"),
            concat!(
                "{\"timestamp\":\"2026-04-01T01:00:00Z\",\"level\":\"INFO\",\"target\":\"gwt\",\"fields\":{\"message\":\"flow_start\",\"category\":\"ui\",\"event\":\"refresh_logs\",\"result\":\"start\",\"workspace\":\"feature-1776\"}}\n",
                "{\"timestamp\":\"2026-04-01T01:00:01Z\",\"level\":\"INFO\",\"target\":\"gwt\",\"fields\":{\"message\":\"flow_success\",\"category\":\"ui\",\"event\":\"refresh_logs\",\"result\":\"success\",\"workspace\":\"feature-1776\"}}\n"
            ),
        )
        .unwrap();
        fs::write(
            other_dir.join("gwt.jsonl.2026-04-01"),
            "{\"timestamp\":\"2026-04-01T02:00:00Z\",\"level\":\"INFO\",\"target\":\"gwt\",\"fields\":{\"message\":\"other\",\"workspace\":\"other-workspace\"}}\n",
        )
        .unwrap();

        let entries = load_log_entries_from_root(temp.path(), Some("feature-1776"));
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|entry| entry.workspace.as_deref() == Some("feature-1776")));
    }

    #[test]
    fn load_log_entries_from_root_falls_back_to_any_workspace() {
        let temp = TempDir::new().unwrap();
        let other_dir = temp.path().join("other-workspace");
        fs::create_dir_all(&other_dir).unwrap();
        fs::write(
            other_dir.join("gwt.jsonl.2026-04-01"),
            "{\"timestamp\":\"2026-04-01T02:00:00Z\",\"level\":\"INFO\",\"target\":\"gwt\",\"fields\":{\"message\":\"other\",\"category\":\"ui\",\"event\":\"switch_management_tab\",\"result\":\"success\"}}\n",
        )
        .unwrap();

        let entries = load_log_entries_from_root(temp.path(), Some("missing-workspace"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].workspace.as_deref(), Some("other-workspace"));
    }
}
