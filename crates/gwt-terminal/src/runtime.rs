//! Host terminal lifecycle and crossterm event normalization.

use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags, MouseEvent, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, Command};

const TICK_RATE: Duration = Duration::from_millis(100);
pub const ESCAPE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(120);

/// Neutral terminal event emitted by the runtime before any TUI-specific
/// routing or `Message` translation happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Key(KeyEvent),
    Paste(String),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    TerminalLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableAlternateScrollMode;

impl Command for DisableAlternateScrollMode {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\u{1b}[?1007l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

pub fn enter_raw_mode() -> io::Result<()> {
    enable_raw_mode()
}

pub fn leave_raw_mode() -> io::Result<()> {
    disable_raw_mode()
}

pub fn enter_terminal(writer: &mut impl io::Write) -> io::Result<()> {
    execute!(
        writer,
        EnterAlternateScreen,
        DisableAlternateScrollMode,
        EnableMouseCapture,
        EnableBracketedPaste,
    )?;
    enable_keyboard_enhancements(writer);
    Ok(())
}

pub fn leave_terminal(writer: &mut impl io::Write) -> io::Result<()> {
    disable_keyboard_enhancements(writer);
    execute!(
        writer,
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
    )
}

pub fn terminal_enter_commands_ansi() -> String {
    let mut ansi = String::new();
    EnterAlternateScreen
        .write_ansi(&mut ansi)
        .expect("enter alternate screen ansi");
    DisableAlternateScrollMode
        .write_ansi(&mut ansi)
        .expect("disable alternate scroll ansi");
    EnableMouseCapture
        .write_ansi(&mut ansi)
        .expect("enable mouse capture ansi");
    EnableBracketedPaste
        .write_ansi(&mut ansi)
        .expect("enable bracketed paste ansi");
    PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
        .write_ansi(&mut ansi)
        .expect("enable keyboard enhancement ansi");
    ansi
}

pub fn terminal_leave_commands_ansi() -> String {
    let mut ansi = String::new();
    PopKeyboardEnhancementFlags
        .write_ansi(&mut ansi)
        .expect("disable keyboard enhancement ansi");
    LeaveAlternateScreen
        .write_ansi(&mut ansi)
        .expect("leave alternate screen ansi");
    DisableMouseCapture
        .write_ansi(&mut ansi)
        .expect("disable mouse capture ansi");
    DisableBracketedPaste
        .write_ansi(&mut ansi)
        .expect("disable bracketed paste ansi");
    ansi
}

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
}

fn enable_keyboard_enhancements(writer: &mut impl io::Write) {
    let _ = execute!(
        writer,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    );
}

fn disable_keyboard_enhancements(writer: &mut impl io::Write) {
    let _ = execute!(writer, PopKeyboardEnhancementFlags);
}

fn stdin_was_initially_terminal() -> bool {
    use std::io::IsTerminal;
    use std::sync::OnceLock;

    static WAS_TERMINAL: OnceLock<bool> = OnceLock::new();
    *WAS_TERMINAL.get_or_init(|| std::io::stdin().is_terminal())
}

/// Check whether the controlling terminal is still alive.
pub fn is_tty_alive() -> bool {
    if !stdin_was_initially_terminal() {
        return true;
    }

    use std::os::unix::io::AsRawFd;

    let fd = std::io::stdin().as_raw_fd();
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) == 0 }
}

pub fn next_tick_deadline() -> Instant {
    next_tick_deadline_from(Instant::now())
}

pub fn next_tick_deadline_from(now: Instant) -> Instant {
    now + TICK_RATE
}

pub fn poll_event(deadline: Instant) -> Option<TerminalEvent> {
    poll_event_slice(deadline, deadline.saturating_duration_since(Instant::now()))
}

pub fn poll_event_slice(deadline: Instant, max_wait: Duration) -> Option<TerminalEvent> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Some(TerminalEvent::Tick);
    }

    if !is_tty_alive() {
        return Some(TerminalEvent::TerminalLost);
    }

    let wait = remaining.min(max_wait);
    match event::poll(wait) {
        Ok(true) => event::read().ok().and_then(translate_crossterm_event),
        Ok(false) => {
            if Instant::now() >= deadline {
                Some(TerminalEvent::Tick)
            } else {
                None
            }
        }
        Err(e) => {
            if stdin_was_initially_terminal() {
                tracing::warn!(error = %e, "crossterm::event::poll returned error");
                Some(TerminalEvent::TerminalLost)
            } else {
                None
            }
        }
    }
}

pub fn translate_crossterm_event(event: Event) -> Option<TerminalEvent> {
    match event {
        Event::Key(key)
            if matches!(
                key.kind,
                event::KeyEventKind::Press | event::KeyEventKind::Repeat
            ) =>
        {
            Some(TerminalEvent::Key(key))
        }
        Event::Paste(text) => Some(TerminalEvent::Paste(text)),
        Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved => None,
        Event::Mouse(mouse) => Some(TerminalEvent::Mouse(mouse)),
        Event::Resize(w, h) => Some(TerminalEvent::Resize(w, h)),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct InputNormalizer {
    pending: VecDeque<TerminalEvent>,
    sgr_mouse_prefix: Option<(Instant, String)>,
}

impl InputNormalizer {
    pub fn pop_pending(&mut self, now: Instant) -> Option<TerminalEvent> {
        self.flush_expired(now);
        self.pending.pop_front()
    }

    pub fn normalize(&mut self, event: TerminalEvent, now: Instant) -> Option<TerminalEvent> {
        self.flush_expired(now);

        match event {
            TerminalEvent::Key(key) => self.normalize_key(key, now),
            other => self.flush_prefix_then(other),
        }
    }

    fn normalize_key(&mut self, key: KeyEvent, now: Instant) -> Option<TerminalEvent> {
        if key.kind != event::KeyEventKind::Press {
            return self.flush_prefix_then_key(key);
        }

        if let Some((last_seen_at, buffer)) = self.sgr_mouse_prefix.as_mut() {
            if now.duration_since(*last_seen_at) > ESCAPE_SEQUENCE_TIMEOUT {
                return self.flush_prefix_then_key(key);
            }

            match key.code {
                KeyCode::Char(ch) => {
                    buffer.push(ch);
                    *last_seen_at = now;
                    if let Some(mouse) = parse_sgr_mouse_report(buffer) {
                        self.sgr_mouse_prefix = None;
                        return Some(TerminalEvent::Mouse(mouse));
                    }
                    if sgr_mouse_report_prefix_is_possible(buffer) {
                        return None;
                    }
                    self.flush_all();
                    self.pending.pop_front()
                }
                _ => self.flush_prefix_then_key(key),
            }
        } else if key.code == KeyCode::Esc && key.modifiers == KeyModifiers::NONE {
            self.sgr_mouse_prefix = Some((now, String::new()));
            None
        } else {
            Some(TerminalEvent::Key(key))
        }
    }

    fn flush_expired(&mut self, now: Instant) {
        if self
            .sgr_mouse_prefix
            .as_ref()
            .is_some_and(|(last_seen_at, _)| {
                now.duration_since(*last_seen_at) > ESCAPE_SEQUENCE_TIMEOUT
            })
        {
            self.flush_all();
        }
    }

    fn flush_all(&mut self) {
        if let Some((_, buffer)) = self.sgr_mouse_prefix.take() {
            self.pending.push_back(TerminalEvent::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )));
            for ch in buffer.chars() {
                self.pending.push_back(TerminalEvent::Key(KeyEvent::new(
                    KeyCode::Char(ch),
                    KeyModifiers::NONE,
                )));
            }
        }
    }

    fn flush_prefix_then(&mut self, event: TerminalEvent) -> Option<TerminalEvent> {
        self.flush_all();
        if self.pending.is_empty() {
            Some(event)
        } else {
            self.pending.push_back(event);
            self.pending.pop_front()
        }
    }

    fn flush_prefix_then_key(&mut self, key: KeyEvent) -> Option<TerminalEvent> {
        self.flush_prefix_then(TerminalEvent::Key(key))
    }
}

fn sgr_mouse_report_prefix_is_possible(buffer: &str) -> bool {
    if !buffer.starts_with('[') {
        return false;
    }
    if buffer == "[" {
        return true;
    }
    if !buffer.starts_with("[<") {
        return false;
    }

    let body = &buffer[2..];
    let mut separators = 0usize;
    for ch in body.chars() {
        match ch {
            '0'..='9' => {}
            ';' => separators += 1,
            'M' | 'm' => return separators == 2,
            _ => return false,
        }
    }

    separators <= 2
}

fn parse_sgr_mouse_report(buffer: &str) -> Option<MouseEvent> {
    let tail = buffer.strip_prefix("[<")?;
    let terminator = tail.chars().last()?;
    if !matches!(terminator, 'M' | 'm') {
        return None;
    }
    let payload = &tail[..tail.len().saturating_sub(1)];
    let mut parts = payload.split(';');
    let cb: u16 = parts.next()?.parse().ok()?;
    let cx: u16 = parts.next()?.parse().ok()?;
    let cy: u16 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let modifiers = decode_sgr_modifiers(cb);
    let column = cx.saturating_sub(1);
    let row = cy.saturating_sub(1);
    let kind = if cb & 64 != 0 {
        match cb & 0b11 {
            0 => MouseEventKind::ScrollUp,
            1 => MouseEventKind::ScrollDown,
            _ => return None,
        }
    } else {
        return None;
    };

    Some(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    })
}

fn decode_sgr_modifiers(cb: u16) -> KeyModifiers {
    let mut modifiers = KeyModifiers::NONE;
    if cb & 4 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if cb & 8 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if cb & 16 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    modifiers
}
