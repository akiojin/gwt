//! ANSI escape sequence stripping for process console lines.
//!
//! Applied in [`super::spawn::forward_stream`] right after
//! [`super::redact::redact_line`] so that lines pushed into the hub
//! ring buffer and broadcast channel are plain text (SPEC-2809 FR-008).
//!
//! Scope:
//!
//! - CSI sequences (`\x1b[...<final byte>`) — colour codes, cursor
//!   moves, erase commands.
//! - OSC sequences (`\x1b]...\x07` or `\x1b]...\x1b\\`) — window
//!   titles, hyperlinks.
//! - SGR aliases handled by the CSI matcher (no separate parser).
//! - Lone `\x1b` followed by a single byte (two-char escape).
//!
//! Out of scope: carriage returns (`\r`) — these are line boundaries
//! that `forward_stream` splits on before redaction; the strip
//! function leaves them untouched in case a caller routes raw text
//! through it.

use std::sync::OnceLock;

use regex::Regex;

/// Replace every ANSI escape sequence in `line` with the empty string.
///
/// Returns a new `String`. When no escape is found, the result is
/// byte-equal to the input.
pub fn strip_ansi(line: &str) -> String {
    ansi_re().replace_all(line, "").into_owned()
}

fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Order matters: try OSC (terminated by BEL or ESC \) first
        // since it can contain `[` which would otherwise be matched as
        // CSI. Then CSI (terminated by 0x40..=0x7E byte). Then two-char
        // escapes.
        Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b[@-_]")
            .expect("ansi escape regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_simple_csi_color() {
        let input = "\x1b[31mred\x1b[0m";
        assert_eq!(strip_ansi(input), "red");
    }

    #[test]
    fn strips_compound_sgr() {
        let input = "\x1b[1;31;47mfoo\x1b[0m bar";
        assert_eq!(strip_ansi(input), "foo bar");
    }

    #[test]
    fn strips_cursor_move() {
        let input = "\x1b[2J\x1b[Hcleared";
        assert_eq!(strip_ansi(input), "cleared");
    }

    #[test]
    fn strips_osc_window_title_bel_terminated() {
        let input = "\x1b]0;my title\x07prompt$ ";
        assert_eq!(strip_ansi(input), "prompt$ ");
    }

    #[test]
    fn strips_osc_string_st_terminated() {
        let input = "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\";
        assert_eq!(strip_ansi(input), "link");
    }

    #[test]
    fn strips_two_char_escape() {
        let input = "before\x1bMafter";
        assert_eq!(strip_ansi(input), "beforeafter");
    }

    #[test]
    fn passes_clean_line_unchanged() {
        let input = "fatal: could not find object";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn preserves_carriage_returns() {
        // `\r` is a line boundary, not an ANSI escape. forward_stream
        // splits on it before redaction; strip_ansi must not consume it.
        let input = "Pulling\r[====>    ] 50%\r[========>] 100%";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn handles_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn strips_mixed_escapes_and_text() {
        let input = "\x1b[32mPulling\x1b[0m \x1b[36mlayer abc\x1b[0m \x1b]0;Docker\x07done";
        assert_eq!(strip_ansi(input), "Pulling layer abc done");
    }

    #[test]
    fn handles_orphan_escape_followed_by_text() {
        // A lone ESC followed by a printable letter is a two-char
        // escape; the byte after ESC is consumed but the rest survives.
        let input = "x\x1bDy";
        assert_eq!(strip_ansi(input), "xy");
    }
}
