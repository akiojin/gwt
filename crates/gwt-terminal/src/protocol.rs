//! Terminal protocol helpers for PTY input encoding.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Returns whether the current vt100 screen requested bracketed paste.
pub fn screen_requests_bracketed_paste(screen: &vt100::Screen) -> bool {
    screen
        .input_mode_formatted()
        .windows(b"\x1b[?2004h".len())
        .any(|window| window == b"\x1b[?2004h")
}

/// Encode a single key event into the PTY byte sequence expected by xterm-like
/// terminals. Returns `None` for events that should not be forwarded.
pub fn key_event_to_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            control_char_bytes(ch)
        }
        KeyCode::Char(ch) => Some(ch.to_string().into_bytes()),
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::F(n) => f_key_to_bytes(n),
        _ => None,
    }
}

/// Encode pasted text for PTY injection, wrapping the payload with bracketed
/// paste delimiters when the application requested that mode.
pub fn build_paste_input_bytes(text: &str, bracketed_paste_enabled: bool) -> Option<Vec<u8>> {
    if text.is_empty() {
        return None;
    }

    if bracketed_paste_enabled {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        Some(bytes)
    } else {
        Some(text.as_bytes().to_vec())
    }
}

fn f_key_to_bytes(n: u8) -> Option<Vec<u8>> {
    match n {
        1 => Some(b"\x1bOP".to_vec()),
        2 => Some(b"\x1bOQ".to_vec()),
        3 => Some(b"\x1bOR".to_vec()),
        4 => Some(b"\x1bOS".to_vec()),
        5 => Some(b"\x1b[15~".to_vec()),
        6 => Some(b"\x1b[17~".to_vec()),
        7 => Some(b"\x1b[18~".to_vec()),
        8 => Some(b"\x1b[19~".to_vec()),
        9 => Some(b"\x1b[20~".to_vec()),
        10 => Some(b"\x1b[21~".to_vec()),
        11 => Some(b"\x1b[23~".to_vec()),
        12 => Some(b"\x1b[24~".to_vec()),
        _ => None,
    }
}

fn control_char_bytes(ch: char) -> Option<Vec<u8>> {
    let ch = ch.to_ascii_lowercase();
    match ch {
        '@' | ' ' => Some(vec![0x00]),
        'a'..='z' => Some(vec![(ch as u8) & 0x1f]),
        '[' => Some(vec![0x1b]),
        '\\' => Some(vec![0x1c]),
        ']' => Some(vec![0x1d]),
        '^' => Some(vec![0x1e]),
        '_' => Some(vec![0x1f]),
        _ => None,
    }
}
