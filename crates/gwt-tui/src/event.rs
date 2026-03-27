use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent};
use tokio::sync::mpsc;

/// Events produced by the TUI event loop.
#[derive(Debug)]
pub enum TuiEvent {
    /// Keyboard input.
    Key(KeyEvent),
    /// Terminal resized.
    Resize(u16, u16),
    /// PTY output from a pane.
    PtyOutput { pane_id: String, data: Vec<u8> },
    /// Periodic tick for UI refresh.
    Tick,
}

/// Sender for PTY output events.
pub type PtyOutputSender = mpsc::UnboundedSender<(String, Vec<u8>)>;

/// Receiver for PTY output events.
pub type PtyOutputReceiver = mpsc::UnboundedReceiver<(String, Vec<u8>)>;

/// Create a PTY output channel pair.
pub fn pty_output_channel() -> (PtyOutputSender, PtyOutputReceiver) {
    mpsc::unbounded_channel()
}

/// Event loop that multiplexes terminal events, PTY output, and tick timer.
pub struct EventLoop {
    pty_rx: PtyOutputReceiver,
    tick_rate: Duration,
}

impl EventLoop {
    /// Create a new event loop with the given PTY output receiver.
    pub fn new(pty_rx: PtyOutputReceiver) -> Self {
        Self {
            pty_rx,
            tick_rate: Duration::from_millis(100),
        }
    }

    /// Poll for the next event. This is a blocking call.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<TuiEvent, Box<dyn std::error::Error>> {
        // Check for PTY output first (non-blocking)
        if let Ok((pane_id, data)) = self.pty_rx.try_recv() {
            return Ok(TuiEvent::PtyOutput { pane_id, data });
        }

        // Poll for crossterm events with tick rate timeout
        if event::poll(self.tick_rate)? {
            match event::read()? {
                Event::Key(key) => Ok(TuiEvent::Key(key)),
                Event::Resize(w, h) => Ok(TuiEvent::Resize(w, h)),
                _ => Ok(TuiEvent::Tick),
            }
        } else {
            Ok(TuiEvent::Tick)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_output_channel_send_receive() {
        let (tx, mut rx) = pty_output_channel();
        tx.send(("pane-1".to_string(), b"hello".to_vec())).unwrap();
        let (id, data) = rx.try_recv().unwrap();
        assert_eq!(id, "pane-1");
        assert_eq!(data, b"hello");
    }

    #[test]
    fn test_event_loop_receives_pty_output() {
        let (tx, rx) = pty_output_channel();
        let mut event_loop = EventLoop::new(rx);
        tx.send(("pane-1".to_string(), b"test data".to_vec()))
            .unwrap();
        let evt = event_loop.next().unwrap();
        match evt {
            TuiEvent::PtyOutput { pane_id, data } => {
                assert_eq!(pane_id, "pane-1");
                assert_eq!(data, b"test data");
            }
            _ => panic!("Expected PtyOutput event"),
        }
    }
}
