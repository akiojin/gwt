use std::{path::PathBuf, time::Duration};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthStr;

pub const DEFAULT_LOG_PATH: &str = "/tmp/gwt-crossterm-events.jsonl";
pub const DEFAULT_TICK_RATE: Duration = Duration::from_millis(100);
pub const INPUT_PROMPT: &str = "Input: ";
const MAX_RECENT_EVENTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeMode {
    Raw,
    Redraw,
    Ratatui,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOptions {
    pub mode: ProbeMode,
    pub output_path: PathBuf,
    pub tick_rate: Duration,
}

#[derive(Debug, Default)]
pub struct ProbeState {
    input_buffer: String,
    submitted_lines: Vec<String>,
    recent_events: Vec<String>,
    last_ctrl_c: bool,
    frame_count: u64,
    event_count: u64,
}

impl ProbeMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "raw" => Ok(Self::Raw),
            "redraw" => Ok(Self::Redraw),
            "ratatui" => Ok(Self::Ratatui),
            _ => Err(format!("unsupported probe mode: {value}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Redraw => "redraw",
            Self::Ratatui => "ratatui",
        }
    }
}

impl ProbeOptions {
    pub fn parse_args<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut mode = ProbeMode::Raw;
        let mut output_path = None;
        let mut tick_rate = DEFAULT_TICK_RATE;
        let mut iter = args.into_iter();

        while let Some(arg) = iter.next() {
            if let Some(value) = arg.strip_prefix("--mode=") {
                mode = ProbeMode::parse(value)?;
                continue;
            }
            if let Some(value) = arg.strip_prefix("--tick-ms=") {
                tick_rate = parse_tick_rate(value)?;
                continue;
            }

            match arg.as_str() {
                "--mode" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--mode requires a value".to_string())?;
                    mode = ProbeMode::parse(&value)?;
                }
                "--tick-ms" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--tick-ms requires a value".to_string())?;
                    tick_rate = parse_tick_rate(&value)?;
                }
                "--help" | "-h" => return Err(help_text().to_string()),
                _ if arg.starts_with('-') => {
                    return Err(format!("unsupported argument: {arg}"));
                }
                _ if output_path.is_none() => {
                    output_path = Some(PathBuf::from(arg));
                }
                _ => {
                    return Err(format!("unexpected positional argument: {arg}"));
                }
            }
        }

        Ok(Self {
            mode,
            output_path: output_path.unwrap_or_else(|| PathBuf::from(DEFAULT_LOG_PATH)),
            tick_rate,
        })
    }

    pub fn needs_periodic_redraw(&self) -> bool {
        !matches!(self.mode, ProbeMode::Raw)
    }
}

impl ProbeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_event(&mut self, event: &Event) {
        self.event_count = self.event_count.saturating_add(1);
        self.recent_events.push(format!("{event:?}"));
        if self.recent_events.len() > MAX_RECENT_EVENTS {
            let overflow = self.recent_events.len() - MAX_RECENT_EVENTS;
            self.recent_events.drain(0..overflow);
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return false;
        }

        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let should_exit = self.last_ctrl_c;
            self.last_ctrl_c = true;
            return should_exit;
        }
        self.last_ctrl_c = false;

        match key.code {
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Enter => {
                self.submitted_lines
                    .push(std::mem::take(&mut self.input_buffer));
            }
            KeyCode::Char(ch) if is_text_input(key.modifiers) => {
                self.input_buffer.push(ch);
            }
            _ => {}
        }

        false
    }

    pub fn cursor_column(&self) -> u16 {
        (UnicodeWidthStr::width(INPUT_PROMPT) + UnicodeWidthStr::width(self.input_buffer.as_str()))
            as u16
    }

    pub fn recent_events(&self) -> &[String] {
        &self.recent_events
    }

    pub fn input_buffer(&self) -> &str {
        &self.input_buffer
    }

    pub fn submitted_lines(&self) -> &[String] {
        &self.submitted_lines
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    pub fn tick_frame(&mut self) {
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn reset_ctrl_c(&mut self) {
        self.last_ctrl_c = false
    }
}

fn parse_tick_rate(value: &str) -> Result<Duration, String> {
    let millis = value
        .parse::<u64>()
        .map_err(|_| format!("invalid tick duration: {value}"))?;
    if millis == 0 {
        return Err("tick duration must be greater than zero".to_string());
    }
    Ok(Duration::from_millis(millis))
}

fn is_text_input(modifiers: KeyModifiers) -> bool {
    !(modifiers.contains(KeyModifiers::CONTROL)
        || modifiers.contains(KeyModifiers::ALT)
        || modifiers.contains(KeyModifiers::SUPER)
        || modifiers.contains(KeyModifiers::HYPER)
        || modifiers.contains(KeyModifiers::META))
}

pub fn help_text() -> &'static str {
    "Usage: cargo run -p gwt-tui --example keytest -- [--mode raw|redraw|ratatui] [--tick-ms N] [output_path]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn parse_args_supports_mode_tick_rate_and_output_path() {
        let options = ProbeOptions::parse_args([
            "--mode".to_string(),
            "ratatui".to_string(),
            "--tick-ms".to_string(),
            "40".to_string(),
            "/tmp/custom-ime-probe.jsonl".to_string(),
        ])
        .expect("parse args");

        assert_eq!(options.mode, ProbeMode::Ratatui);
        assert_eq!(options.tick_rate, Duration::from_millis(40));
        assert_eq!(
            options.output_path,
            PathBuf::from("/tmp/custom-ime-probe.jsonl")
        );
    }

    #[test]
    fn parse_args_defaults_to_raw_mode_and_default_output_path() {
        let options = ProbeOptions::parse_args(std::iter::empty::<String>()).expect("parse args");

        assert_eq!(options.mode, ProbeMode::Raw);
        assert_eq!(options.tick_rate, DEFAULT_TICK_RATE);
        assert_eq!(options.output_path, PathBuf::from(DEFAULT_LOG_PATH));
    }

    #[test]
    fn probe_state_keeps_recent_events_bounded() {
        let mut state = ProbeState::new();
        for index in 0..12 {
            state.record_event(&Event::Key(key(
                KeyCode::Char(char::from(b'a' + index as u8)),
                KeyModifiers::NONE,
            )));
        }

        let recent = state.recent_events();
        assert_eq!(recent.len(), 8);
        assert!(recent
            .first()
            .is_some_and(|line| line.contains("Char('e')")));
        assert!(recent.last().is_some_and(|line| line.contains("Char('l')")));
    }

    #[test]
    fn probe_state_edits_committed_buffer_and_uses_display_width_for_cursor() {
        let mut state = ProbeState::new();
        assert!(!state.handle_key_event(key(KeyCode::Char('a'), KeyModifiers::NONE)));
        assert!(!state.handle_key_event(key(KeyCode::Char('気'), KeyModifiers::NONE)));
        assert_eq!(state.input_buffer(), "a気");
        assert_eq!(state.cursor_column(), 10);

        assert!(!state.handle_key_event(key(KeyCode::Backspace, KeyModifiers::NONE)));
        assert_eq!(state.input_buffer(), "a");
        assert_eq!(state.cursor_column(), 8);
    }

    #[test]
    fn probe_state_requires_double_ctrl_c_to_exit() {
        let mut state = ProbeState::new();
        assert!(!state.handle_key_event(key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(state.handle_key_event(key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }
}
