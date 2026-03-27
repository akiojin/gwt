//! TUI event types and PTY output channel

use crossterm::event::KeyEvent;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum TuiEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    PtyOutput { pane_id: String, data: Vec<u8> },
    Tick,
}

pub type PtyOutputSender = mpsc::UnboundedSender<(String, Vec<u8>)>;
pub type PtyOutputReceiver = mpsc::UnboundedReceiver<(String, Vec<u8>)>;

pub fn pty_output_channel() -> (PtyOutputSender, PtyOutputReceiver) {
    mpsc::unbounded_channel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_output_channel_send_recv() {
        let (tx, mut rx) = pty_output_channel();
        tx.send(("pane-1".to_string(), vec![0x41, 0x42]))
            .expect("send should succeed");
        let (id, data) = rx.try_recv().expect("should receive");
        assert_eq!(id, "pane-1");
        assert_eq!(data, vec![0x41, 0x42]);
    }
}
