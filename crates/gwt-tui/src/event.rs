//! Event adapter — maps neutral terminal runtime events into TUI messages.

use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use gwt_terminal::runtime::{self, TerminalEvent};

use crate::{input_trace, message::Message};

pub use gwt_terminal::runtime::{next_tick_deadline, next_tick_deadline_from, InputNormalizer};

/// Drain a pending normalized runtime event and translate it into a TUI
/// message, if one is ready.
pub fn pop_pending_message(normalizer: &mut InputNormalizer, now: Instant) -> Option<Message> {
    normalizer
        .pop_pending(now)
        .and_then(translate_terminal_event)
}

/// Poll the runtime until the next deadline and translate the next available
/// normalized runtime event into a TUI message.
pub fn poll_event(deadline: Instant, normalizer: &mut InputNormalizer) -> Option<Message> {
    poll_event_slice(
        deadline,
        deadline.saturating_duration_since(Instant::now()),
        normalizer,
    )
}

/// Poll the runtime with a capped wait time and translate the next available
/// normalized runtime event into a TUI message.
pub fn poll_event_slice(
    deadline: Instant,
    max_wait: Duration,
    normalizer: &mut InputNormalizer,
) -> Option<Message> {
    loop {
        let event = runtime::poll_event_slice(deadline, max_wait)?;
        if let Some(message) = normalize_terminal_event(normalizer, event, Instant::now()) {
            return Some(message);
        }
    }
}

/// Normalize a runtime event and map it to a TUI `Message`.
pub fn normalize_terminal_event(
    normalizer: &mut InputNormalizer,
    event: TerminalEvent,
    now: Instant,
) -> Option<Message> {
    normalizer
        .normalize(event, now)
        .and_then(translate_terminal_event)
}

/// Translate a runtime event into a TUI `Message`.
pub fn translate_terminal_event(event: TerminalEvent) -> Option<Message> {
    match event {
        TerminalEvent::Key(key) => {
            input_trace::trace_crossterm_key(key);
            Some(Message::KeyInput(key))
        }
        TerminalEvent::Paste(text) => Some(Message::PasteInput(text)),
        TerminalEvent::Mouse(mouse) => Some(Message::MouseInput(mouse)),
        TerminalEvent::Resize(w, h) => Some(Message::Resize(w, h)),
        TerminalEvent::Tick => Some(Message::Tick),
        TerminalEvent::TerminalLost => Some(Message::TerminalLost),
    }
}

/// Convert a raw key event into a high-level Message via the keybind system,
/// or return it as-is for PTY forwarding.
pub fn classify_key(key: KeyEvent) -> Message {
    Message::KeyInput(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers, MouseEvent};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn translate_terminal_event_maps_key_to_message() {
        let message = translate_terminal_event(TerminalEvent::Key(key(KeyCode::Tab)));
        assert!(matches!(
            message,
            Some(Message::KeyInput(KeyEvent {
                code: KeyCode::Tab,
                ..
            }))
        ));
    }

    #[test]
    fn translate_terminal_event_maps_paste_resize_tick_and_lost() {
        assert!(matches!(
            translate_terminal_event(TerminalEvent::Paste("git status".into())),
            Some(Message::PasteInput(text)) if text == "git status"
        ));
        assert!(matches!(
            translate_terminal_event(TerminalEvent::Resize(80, 24)),
            Some(Message::Resize(80, 24))
        ));
        assert!(matches!(
            translate_terminal_event(TerminalEvent::Tick),
            Some(Message::Tick)
        ));
        assert!(matches!(
            translate_terminal_event(TerminalEvent::TerminalLost),
            Some(Message::TerminalLost)
        ));
    }

    #[test]
    fn translate_terminal_event_maps_mouse_to_message() {
        let mouse = MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let message = translate_terminal_event(TerminalEvent::Mouse(mouse));
        assert!(matches!(
            message,
            Some(Message::MouseInput(MouseEvent {
                kind: crossterm::event::MouseEventKind::ScrollUp,
                column: 10,
                row: 5,
                ..
            }))
        ));
    }

    #[test]
    fn pop_pending_message_drains_runtime_normalizer() {
        let mut normalizer = InputNormalizer::default();
        let now = Instant::now();

        assert!(normalizer
            .normalize(TerminalEvent::Key(key(KeyCode::Esc)), now)
            .is_none());
        let message = pop_pending_message(
            &mut normalizer,
            now + runtime::ESCAPE_SEQUENCE_TIMEOUT + Duration::from_millis(1),
        );
        assert!(matches!(
            message,
            Some(Message::KeyInput(KeyEvent {
                code: KeyCode::Esc,
                ..
            }))
        ));
    }

    #[test]
    fn poll_event_slice_returns_none_before_deadline_timeout() {
        let mut normalizer = InputNormalizer::default();
        let deadline = Instant::now() + Duration::from_secs(1);
        let message = poll_event_slice(deadline, Duration::ZERO, &mut normalizer);
        assert!(message.is_none());
    }
}
