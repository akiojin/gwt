use std::time::{Duration, Instant};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseEvent, MouseEventKind,
};
use gwt_terminal::protocol::{
    build_paste_input_bytes, key_event_to_bytes, screen_requests_bracketed_paste,
};
use gwt_terminal::runtime::{
    next_tick_deadline, terminal_enter_commands_ansi, terminal_leave_commands_ansi,
    translate_crossterm_event, InputNormalizer, TerminalEvent, ESCAPE_SEQUENCE_TIMEOUT,
};

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
    assert!(deadline - now <= Duration::from_millis(105));
}

#[test]
fn terminal_enter_commands_disable_alternate_scroll_and_enable_keyboard_enhancements() {
    let ansi = terminal_enter_commands_ansi();
    assert!(ansi.contains("\u{1b}[?1007l"));
    assert!(ansi.contains("\u{1b}[?2004h"));
    assert!(ansi.contains("\u{1b}[?1000h") || ansi.contains("\u{1b}[?1002h"));
}

#[test]
fn terminal_leave_commands_disable_bracketed_paste() {
    let ansi = terminal_leave_commands_ansi();
    assert!(ansi.contains("\u{1b}[?2004l"));
}

#[test]
fn translate_crossterm_event_maps_repeat_key_to_terminal_event() {
    let event = translate_crossterm_event(Event::Key(key_with_kind(
        KeyCode::Tab,
        KeyEventKind::Repeat,
    )));
    assert!(matches!(
        event,
        Some(TerminalEvent::Key(key))
            if key.code == KeyCode::Tab && key.kind == KeyEventKind::Repeat
    ));
}

#[test]
fn translate_crossterm_event_maps_paste_to_terminal_event() {
    let event = translate_crossterm_event(Event::Paste("git status\npwd".into()));
    assert!(matches!(
        event,
        Some(TerminalEvent::Paste(text)) if text == "git status\npwd"
    ));
}

#[test]
fn translate_crossterm_event_ignores_mouse_move_and_key_release() {
    let moved = translate_crossterm_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(moved.is_none());

    let released = translate_crossterm_event(Event::Key(key_with_kind(
        KeyCode::Tab,
        KeyEventKind::Release,
    )));
    assert!(released.is_none());
}

#[test]
fn input_normalizer_converts_leaked_sgr_wheel_report_to_mouse_input() {
    let mut normalizer = InputNormalizer::default();
    let now = Instant::now();

    assert!(normalizer
        .normalize(TerminalEvent::Key(key(KeyCode::Esc)), now)
        .is_none());
    assert!(normalizer
        .normalize(
            TerminalEvent::Key(key(KeyCode::Char('['))),
            now + Duration::from_millis(1),
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
                TerminalEvent::Key(key(KeyCode::Char(ch))),
                now + Duration::from_millis(offset_ms),
            )
            .is_none());
    }

    let event = normalizer.normalize(
        TerminalEvent::Key(key(KeyCode::Char('M'))),
        now + Duration::from_millis(12),
    );
    assert!(matches!(
        event,
        Some(TerminalEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 174,
            row: 42,
            modifiers
        })) if modifiers == KeyModifiers::NONE
    ));
}

#[test]
fn input_normalizer_releases_plain_escape_after_timeout() {
    let mut normalizer = InputNormalizer::default();
    let now = Instant::now();

    assert!(normalizer
        .normalize(TerminalEvent::Key(key(KeyCode::Esc)), now)
        .is_none());
    let flushed = normalizer.pop_pending(now + ESCAPE_SEQUENCE_TIMEOUT + Duration::from_millis(1));
    assert!(matches!(
        flushed,
        Some(TerminalEvent::Key(KeyEvent {
            code: KeyCode::Esc,
            ..
        }))
    ));
}

#[test]
fn input_normalizer_replays_invalid_escape_prefix_in_original_order() {
    let mut normalizer = InputNormalizer::default();
    let now = Instant::now();

    assert!(normalizer
        .normalize(TerminalEvent::Key(key(KeyCode::Esc)), now)
        .is_none());
    assert!(normalizer
        .normalize(
            TerminalEvent::Key(key(KeyCode::Char('['))),
            now + Duration::from_millis(1),
        )
        .is_none());

    let first = normalizer.normalize(
        TerminalEvent::Key(key(KeyCode::Char('j'))),
        now + Duration::from_millis(2),
    );
    let second = normalizer.pop_pending(now + Duration::from_millis(3));
    let third = normalizer.pop_pending(now + Duration::from_millis(4));

    assert!(matches!(
        first,
        Some(TerminalEvent::Key(KeyEvent {
            code: KeyCode::Esc,
            ..
        }))
    ));
    assert!(matches!(
        second,
        Some(TerminalEvent::Key(KeyEvent {
            code: KeyCode::Char('['),
            ..
        }))
    ));
    assert!(matches!(
        third,
        Some(TerminalEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            ..
        }))
    ));
}

#[test]
fn key_event_to_bytes_maps_control_c_and_backtab() {
    assert_eq!(
        key_event_to_bytes(key(KeyCode::Char('c'))),
        Some(b"c".to_vec())
    );
    assert_eq!(
        key_event_to_bytes(KeyEvent {
            modifiers: KeyModifiers::CONTROL,
            ..key(KeyCode::Char('c'))
        }),
        Some(vec![0x03])
    );
    assert_eq!(
        key_event_to_bytes(KeyEvent {
            modifiers: KeyModifiers::SHIFT,
            ..key(KeyCode::BackTab)
        }),
        Some(b"\x1b[Z".to_vec())
    );
}

#[test]
fn build_paste_input_bytes_wraps_payload_when_bracketed_paste_is_enabled() {
    let bytes = build_paste_input_bytes("git status\npwd", true).unwrap();
    assert_eq!(bytes, b"\x1b[200~git status\npwd\x1b[201~".to_vec());
}

#[test]
fn build_paste_input_bytes_preserves_plain_and_whitespace_payloads() {
    assert_eq!(
        build_paste_input_bytes("echo hello", false),
        Some(b"echo hello".to_vec())
    );
    assert_eq!(
        build_paste_input_bytes("   \n", false),
        Some(b"   \n".to_vec())
    );
    assert_eq!(build_paste_input_bytes("", false), None);
}

#[test]
fn screen_requests_bracketed_paste_tracks_vt100_mode() {
    let mut parser = vt100::Parser::new(24, 80, 0);
    assert!(!screen_requests_bracketed_paste(parser.screen()));

    parser.process(b"\x1b[?2004h");
    assert!(screen_requests_bracketed_paste(parser.screen()));

    parser.process(b"\x1b[?2004l");
    assert!(!screen_requests_bracketed_paste(parser.screen()));
}
