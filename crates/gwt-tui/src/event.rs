//! Event loop: polls crossterm events and PTY output into a unified stream

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent, KeyEventKind, MouseEvent};

/// Unified TUI event
#[derive(Debug)]
pub enum TuiEvent {
    /// Key press event (only Press kind, not Release/Repeat)
    Key(KeyEvent),
    /// Pasted text from the terminal
    Paste(String),
    /// Mouse event
    Mouse(MouseEvent),
    /// Terminal resize
    Resize(u16, u16),
    /// PTY output from a pane
    PtyOutput { pane_id: String, data: Vec<u8> },
    /// Tick for background polling
    Tick,
}

/// PTY output message sent from reader threads to the event loop.
#[derive(Debug)]
pub struct PtyOutputMsg {
    pub pane_id: String,
    pub data: Vec<u8>,
}

/// Sender half for PTY output (cloned per reader thread).
pub type PtyOutputSender = Sender<PtyOutputMsg>;
/// Receiver half consumed by the event loop.
pub type PtyOutputReceiver = Receiver<PtyOutputMsg>;

/// Create a channel pair for PTY output.
pub fn pty_output_channel() -> (PtyOutputSender, PtyOutputReceiver) {
    mpsc::channel()
}

/// Event loop that merges crossterm events with PTY output.
pub struct EventLoop {
    pty_rx: PtyOutputReceiver,
    poll_timeout: Duration,
}

impl EventLoop {
    /// Create a new event loop with the PTY output receiver.
    pub fn new(pty_rx: PtyOutputReceiver) -> Self {
        Self {
            pty_rx,
            // Poll crossterm at 50ms intervals for responsive PTY output
            poll_timeout: Duration::from_millis(50),
        }
    }

    fn read_ready_terminal_event(
        &self,
        timeout: Duration,
    ) -> Result<Option<TuiEvent>, Box<dyn std::error::Error>> {
        if !event::poll(timeout)? {
            return Ok(None);
        }

        let event = match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => Some(TuiEvent::Key(key)),
            Event::Paste(data) => Some(TuiEvent::Paste(data)),
            Event::Mouse(mouse) => Some(TuiEvent::Mouse(mouse)),
            Event::Resize(w, h) => Some(TuiEvent::Resize(w, h)),
            _ => None,
        };

        Ok(event)
    }

    fn try_recv_pty(&self) -> Option<TuiEvent> {
        match self.pty_rx.try_recv() {
            Ok(msg) => Some(TuiEvent::PtyOutput {
                pane_id: msg.pane_id,
                data: msg.data,
            }),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Block until the next event is available.
    pub fn next(&self) -> Result<TuiEvent, Box<dyn std::error::Error>> {
        // Prefer already-ready terminal input over PTY output so copy-mode
        // scrolling and selection don't get starved by chatty panes.
        if let Some(event) = self.read_ready_terminal_event(Duration::ZERO)? {
            return Ok(event);
        }

        if let Some(event) = self.try_recv_pty() {
            return Ok(event);
        }

        if let Some(event) = self.read_ready_terminal_event(self.poll_timeout)? {
            return Ok(event);
        }

        if let Some(event) = self.try_recv_pty() {
            return Ok(event);
        }

        // Nothing happened → tick
        Ok(TuiEvent::Tick)
    }
}
