//! Event loop — polls crossterm events, PTY output, and tick timer.

use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent};

use crate::message::Message;

/// Tick interval for the event loop.
const TICK_RATE: Duration = Duration::from_millis(100);

/// Poll for the next message. Returns `None` on timeout with no events.
pub fn poll_event(deadline: Instant) -> Option<Message> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Some(Message::Tick);
    }

    if event::poll(remaining).unwrap_or(false) {
        match event::read() {
            Ok(Event::Key(key)) => Some(Message::KeyInput(key)),
            Ok(Event::Mouse(mouse)) => Some(Message::MouseInput(mouse)),
            Ok(Event::Resize(w, h)) => Some(Message::Resize(w, h)),
            _ => None,
        }
    } else {
        Some(Message::Tick)
    }
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
}
