//! Event loop — polls crossterm events, PTY output, and tick timer.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::{input_trace, message::Message};

/// Tick interval for the event loop.
const TICK_RATE: Duration = Duration::from_millis(100);
const ESCAPE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(120);

/// Poll for the next message. Returns `None` on timeout with no events.
pub fn poll_event(deadline: Instant) -> Option<Message> {
    poll_event_slice(deadline, deadline.saturating_duration_since(Instant::now()))
}

/// Poll for the next message while capping the blocking wait to `max_wait`.
///
/// Returns `None` when the slice times out before the next tick deadline.
pub fn poll_event_slice(deadline: Instant, max_wait: Duration) -> Option<Message> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Some(Message::Tick);
    }

    let wait = remaining.min(max_wait);
    if event::poll(wait).unwrap_or(false) {
        event::read().ok().and_then(translate_event)
    } else if Instant::now() >= deadline {
        Some(Message::Tick)
    } else {
        None
    }
}

fn translate_event(event: Event) -> Option<Message> {
    match event {
        Event::Key(key)
            if matches!(
                key.kind,
                event::KeyEventKind::Press | event::KeyEventKind::Repeat
            ) =>
        {
            input_trace::trace_crossterm_key(key);
            Some(Message::KeyInput(key))
        }
        Event::Paste(text) => Some(Message::PasteInput(text)),
        Event::Mouse(mouse) if mouse.kind == event::MouseEventKind::Moved => None,
        Event::Mouse(mouse) => Some(Message::MouseInput(mouse)),
        Event::Resize(w, h) => Some(Message::Resize(w, h)),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct InputNormalizer {
    pending: VecDeque<Message>,
    sgr_mouse_prefix: Option<(Instant, String)>,
}

impl InputNormalizer {
    pub fn pop_pending(&mut self, now: Instant) -> Option<Message> {
        self.flush_expired(now);
        self.pending.pop_front()
    }

    pub fn normalize(
        &mut self,
        msg: Message,
        now: Instant,
        _terminal_focused: bool,
    ) -> Option<Message> {
        self.flush_expired(now);

        match msg {
            Message::KeyInput(key) => self.normalize_key(key, now),
            other => self.flush_prefix_then(other),
        }
    }

    fn normalize_key(&mut self, key: KeyEvent, now: Instant) -> Option<Message> {
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
                        return Some(Message::MouseInput(mouse));
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
            Some(Message::KeyInput(key))
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
            self.pending.push_back(Message::KeyInput(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )));
            for ch in buffer.chars() {
                self.pending.push_back(Message::KeyInput(KeyEvent::new(
                    KeyCode::Char(ch),
                    KeyModifiers::NONE,
                )));
            }
        }
    }

    fn flush_prefix_then(&mut self, msg: Message) -> Option<Message> {
        self.flush_all();
        if self.pending.is_empty() {
            Some(msg)
        } else {
            self.pending.push_back(msg);
            self.pending.pop_front()
        }
    }

    fn flush_prefix_then_key(&mut self, key: KeyEvent) -> Option<Message> {
        self.flush_prefix_then(Message::KeyInput(key))
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

/// Calculate the next tick deadline.
pub fn next_tick_deadline() -> Instant {
    Instant::now() + TICK_RATE
}

/// Convert a raw key event into a high-level Message via the keybind system,
/// or return it as-is for PTY forwarding.
pub fn classify_key(key: KeyEvent) -> Message {
    // Phase 2: integrate with keybind registry for Ctrl+G prefix
    Message::KeyInput(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn key_with_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent { kind, ..key(code) }
    }

    #[test]
    fn next_tick_deadline_is_in_the_future() {
        let now = Instant::now();
        let deadline = next_tick_deadline();
        assert!(deadline > now);
        assert!(deadline - now <= TICK_RATE + Duration::from_millis(5));
    }

    #[test]
    fn poll_event_returns_tick_on_expired_deadline() {
        let past = Instant::now() - Duration::from_secs(1);
        let msg = poll_event(past);
        assert!(matches!(msg, Some(Message::Tick)));
    }

    #[test]
    fn poll_event_slice_returns_none_before_deadline_timeout() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let msg = poll_event_slice(deadline, Duration::ZERO);
        assert!(msg.is_none());
    }

    #[test]
    fn translate_event_maps_paste_to_message() {
        let msg = translate_event(Event::Paste("git status\npwd".into()));
        assert!(matches!(
            msg,
            Some(Message::PasteInput(text)) if text == "git status\npwd"
        ));
    }

    #[test]
    fn translate_event_maps_repeat_key_to_message() {
        let msg = translate_event(Event::Key(key_with_kind(
            KeyCode::Tab,
            KeyEventKind::Repeat,
        )));
        assert!(matches!(
            msg,
            Some(Message::KeyInput(key))
                if key.code == KeyCode::Tab && key.kind == KeyEventKind::Repeat
        ));
    }

    #[test]
    fn translate_event_ignores_mouse_move_events() {
        let msg = translate_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(msg.is_none());
    }

    #[test]
    fn input_normalizer_converts_leaked_sgr_wheel_report_to_mouse_input() {
        let mut normalizer = InputNormalizer::default();
        let now = Instant::now();

        assert!(normalizer
            .normalize(Message::KeyInput(key(KeyCode::Esc)), now, true)
            .is_none());
        assert!(normalizer
            .normalize(
                Message::KeyInput(key(KeyCode::Char('['))),
                now + Duration::from_millis(1),
                true,
            )
            .is_none());

        for (offset_ms, ch) in [
            (2, '<'),
            (3, '6'),
            (4, '4'),
            (5, ';'),
            (6, '1'),
            (7, '7'),
            (8, '5'),
            (9, ';'),
            (10, '4'),
            (11, '3'),
        ] {
            assert!(normalizer
                .normalize(
                    Message::KeyInput(key(KeyCode::Char(ch))),
                    now + Duration::from_millis(offset_ms),
                    true,
                )
                .is_none());
        }

        let msg = normalizer.normalize(
            Message::KeyInput(key(KeyCode::Char('M'))),
            now + Duration::from_millis(12),
            true,
        );
        assert!(matches!(
            msg,
            Some(Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 174,
                row: 42,
                modifiers
            })) if modifiers == KeyModifiers::NONE
        ));
    }

    #[test]
    fn translate_event_ignores_key_release() {
        let msg = translate_event(Event::Key(key_with_kind(
            KeyCode::Tab,
            KeyEventKind::Release,
        )));
        assert!(msg.is_none());
    }

    #[test]
    fn input_normalizer_releases_plain_escape_after_timeout() {
        let mut normalizer = InputNormalizer::default();
        let now = Instant::now();

        assert!(normalizer
            .normalize(Message::KeyInput(key(KeyCode::Esc)), now, true)
            .is_none());
        let flushed =
            normalizer.pop_pending(now + ESCAPE_SEQUENCE_TIMEOUT + Duration::from_millis(1));
        assert!(matches!(
            flushed,
            Some(Message::KeyInput(KeyEvent {
                code: KeyCode::Esc,
                ..
            }))
        ));
    }

    #[test]
    fn input_normalizer_keeps_parsing_when_inter_char_gaps_stay_within_timeout() {
        let mut normalizer = InputNormalizer::default();
        let now = Instant::now();

        assert!(normalizer
            .normalize(Message::KeyInput(key(KeyCode::Esc)), now, true)
            .is_none());

        for (offset_ms, ch) in [
            (10, '['),
            (20, '<'),
            (30, '6'),
            (40, '4'),
            (50, ';'),
            (60, '1'),
            (70, '7'),
            (80, '5'),
            (90, ';'),
            (100, '4'),
            (110, '3'),
        ] {
            assert!(normalizer
                .normalize(
                    Message::KeyInput(key(KeyCode::Char(ch))),
                    now + Duration::from_millis(offset_ms),
                    true,
                )
                .is_none());
        }

        let msg = normalizer.normalize(
            Message::KeyInput(key(KeyCode::Char('M'))),
            now + Duration::from_millis(120),
            true,
        );
        assert!(matches!(
            msg,
            Some(Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 174,
                row: 42,
                modifiers
            })) if modifiers == KeyModifiers::NONE
        ));
    }

    #[test]
    fn input_normalizer_parses_sgr_wheel_even_when_terminal_is_not_focused() {
        let mut normalizer = InputNormalizer::default();
        let now = Instant::now();

        assert!(normalizer
            .normalize(Message::KeyInput(key(KeyCode::Esc)), now, false)
            .is_none());
        assert!(normalizer
            .normalize(
                Message::KeyInput(key(KeyCode::Char('['))),
                now + Duration::from_millis(5),
                false,
            )
            .is_none());

        for (offset_ms, ch) in [
            (10, '<'),
            (15, '6'),
            (20, '4'),
            (25, ';'),
            (30, '1'),
            (35, '7'),
            (40, '5'),
            (45, ';'),
            (50, '4'),
            (55, '3'),
        ] {
            assert!(normalizer
                .normalize(
                    Message::KeyInput(key(KeyCode::Char(ch))),
                    now + Duration::from_millis(offset_ms),
                    false,
                )
                .is_none());
        }

        let msg = normalizer.normalize(
            Message::KeyInput(key(KeyCode::Char('M'))),
            now + Duration::from_millis(60),
            false,
        );
        assert!(matches!(
            msg,
            Some(Message::MouseInput(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 174,
                row: 42,
                modifiers
            })) if modifiers == KeyModifiers::NONE
        ));
    }

    #[test]
    fn input_normalizer_replays_invalid_escape_prefix_in_original_order() {
        let mut normalizer = InputNormalizer::default();
        let now = Instant::now();

        assert!(normalizer
            .normalize(Message::KeyInput(key(KeyCode::Esc)), now, true)
            .is_none());
        assert!(normalizer
            .normalize(
                Message::KeyInput(key(KeyCode::Char('['))),
                now + Duration::from_millis(1),
                true,
            )
            .is_none());

        let first = normalizer.normalize(
            Message::KeyInput(key(KeyCode::Char('j'))),
            now + Duration::from_millis(2),
            true,
        );
        let second = normalizer.pop_pending(now + Duration::from_millis(3));
        let third = normalizer.pop_pending(now + Duration::from_millis(4));

        assert!(matches!(
            first,
            Some(Message::KeyInput(KeyEvent {
                code: KeyCode::Esc,
                ..
            }))
        ));
        assert!(matches!(
            second,
            Some(Message::KeyInput(KeyEvent {
                code: KeyCode::Char('['),
                ..
            }))
        ));
        assert!(matches!(
            third,
            Some(Message::KeyInput(KeyEvent {
                code: KeyCode::Char('j'),
                ..
            }))
        ));
    }
}
