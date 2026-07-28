//! Terminal pane: integrates PTY handle + vt100 parser + scrollback.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::{
    pty::{PtyHandle, SpawnConfig},
    scrollback::{ScrollbackLine, ScrollbackStorage},
    TerminalError,
};

/// Status of a pane's child process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneStatus {
    Running,
    Completed(i32),
    Error(String),
}

const SNAPSHOT_SCROLLBACK_REPLAY_LIMIT: usize = 5_000;

/// A terminal pane integrating PTY, vt100 parser, and scrollback.
///
/// `pty` is wrapped in an `Arc` so that callers who only need to write input
/// or query process state can hold a lock-free clone without contending with
/// the reader thread's exclusive `Mutex<Pane>` guard. The gwt GUI binary uses
/// this to bypass the tao event loop for `terminal_input` hot path (see the
/// fast-path write in `client_session`).
pub struct Pane {
    id: String,
    pty: Arc<PtyHandle>,
    parser: vt100::Parser,
    scrollback: ScrollbackStorage,
    status: PaneStatus,
    /// Accumulator for incomplete lines from raw PTY output. Holds raw bytes
    /// (including SGR escape sequences) until a `\n` boundary is reached, then
    /// the completed line is split off and pushed into `scrollback` with both
    /// a plain-text rendering and the original byte stream so SGR formatting
    /// can be replayed later (SPEC-1919 FR-003j).
    line_buf: Vec<u8>,
}

fn resize_parser_preserving_state(parser: &mut vt100::Parser, rows: u16, cols: u16) {
    parser.screen_mut().set_size(rows, cols);
}

impl Pane {
    /// Create a new pane by spawning a PTY process.
    pub fn new(
        id: String,
        command: String,
        args: Vec<String>,
        cols: u16,
        rows: u16,
        env: HashMap<String, String>,
        cwd: Option<PathBuf>,
    ) -> Result<Self, TerminalError> {
        Self::new_with_spawn_config(
            id,
            SpawnConfig {
                command,
                args,
                cols,
                rows,
                env,
                remove_env: Vec::new(),
                cwd,
            },
        )
    }

    /// Create a new pane from a fully resolved PTY spawn configuration.
    pub fn new_with_spawn_config(id: String, config: SpawnConfig) -> Result<Self, TerminalError> {
        let rows = config.rows;
        let cols = config.cols;
        let pty = Arc::new(PtyHandle::spawn(config)?);
        let parser = vt100::Parser::new(rows, cols, SNAPSHOT_SCROLLBACK_REPLAY_LIMIT);
        let scrollback = ScrollbackStorage::new(ScrollbackStorage::DEFAULT_CAPACITY);

        Ok(Self {
            id,
            pty,
            parser,
            scrollback,
            status: PaneStatus::Running,
            line_buf: Vec::new(),
        })
    }

    /// Get the pane ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get a reference to the PTY handle.
    pub fn pty(&self) -> &PtyHandle {
        &self.pty
    }

    /// Get a shared handle to the underlying PTY.
    ///
    /// Callers on threads that do not own the surrounding `Mutex<Pane>` guard
    /// can clone this `Arc` and invoke `write_input` / `resize` / `process_id`
    /// without contending with the reader thread.
    pub fn shared_pty(&self) -> Arc<PtyHandle> {
        Arc::clone(&self.pty)
    }

    /// Feed raw bytes from PTY output through the vt100 parser and scrollback.
    ///
    /// The vt100 parser is the single source of truth for terminal screen state.
    /// Completed lines (delimited by `\n`) are also captured into the scrollback
    /// ring buffer for history access.
    pub fn process_bytes(&mut self, data: &[u8]) {
        // Update vt100 screen state
        self.parser.process(data);

        // Capture raw bytes for scrollback. SGR escape sequences (CSI ... m)
        // never contain `\n`, so byte-level newline splitting preserves both
        // the visible text and the SGR formatting in `formatted`.
        self.line_buf.extend_from_slice(data);

        while let Some(pos) = self.line_buf.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = self.line_buf.drain(..pos).collect();
            self.line_buf.drain(..1); // consume the '\n'
            let text = String::from_utf8_lossy(&raw).into_owned();
            self.scrollback.push_line(ScrollbackLine {
                text,
                formatted: raw,
                wrapped: false,
            });
        }
    }

    /// Get the current vt100 screen.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Build a replayable terminal snapshot for frontend reconnect.
    ///
    /// The snapshot is serialized from parsed vt100 state rather than raw PTY
    /// fragments, preserving cursor, erase, wrapping, styling, and alternate
    /// screen semantics without replaying external control-sequence side
    /// effects.
    ///
    /// A terminal byte stream has only one saved-cursor slot per active
    /// buffer, so it cannot simultaneously reproduce distinct saved
    /// cursor/attributes and a pending-wrap cursor over a blank cell. In that
    /// compound state, the snapshot preserves the current frame, cursor,
    /// attributes, and next-printable continuation exactly; saved
    /// cursor/attributes are best effort. Representable saved states remain
    /// exact.
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        self.parser
            .screen()
            .snapshot_formatted(SNAPSHOT_SCROLLBACK_REPLAY_LIMIT)
    }

    /// Get scrollback lines from the ring buffer.
    pub fn scrollback_lines(&self, start: usize, count: usize) -> Vec<&ScrollbackLine> {
        self.scrollback.get_lines(start, count)
    }

    /// Total number of lines in scrollback.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Get the current pane status.
    pub fn status(&self) -> &PaneStatus {
        &self.status
    }

    /// Check and update the pane's process status.
    pub fn check_status(&mut self) -> Result<&PaneStatus, TerminalError> {
        if self.status == PaneStatus::Running {
            if let Some(exit_status) = self.pty.try_wait()? {
                if exit_status.success() {
                    self.status = PaneStatus::Completed(0);
                } else {
                    self.status = PaneStatus::Completed(1);
                }
            }
        }
        Ok(&self.status)
    }

    /// Mark this pane as errored.
    pub fn mark_error(&mut self, message: impl Into<String>) {
        self.status = PaneStatus::Error(message.into());
    }

    /// Write input to the PTY.
    pub fn write_input(&self, data: &[u8]) -> Result<(), TerminalError> {
        self.pty.write_input(data)
    }

    /// Resize the pane (PTY + vt100 parser).
    ///
    /// Emits an `info` event at `target = gwt::resize::pane` capturing the
    /// requested dimensions and total wall time so SPEC-2014 Phase C can tell
    /// PTY ConPTY stalls (logged at `gwt::resize::pty`) apart from
    /// `resize_parser_preserving_state` regressions inside the parser.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let started = std::time::Instant::now();
        self.pty.resize(cols, rows)?;
        let pty_elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let parser_started = std::time::Instant::now();
        resize_parser_preserving_state(&mut self.parser, rows, cols);
        let parser_elapsed_ms =
            u64::try_from(parser_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let total_elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::info!(
            target: "gwt::resize::pane",
            cols = cols,
            rows = rows,
            pty_elapsed_ms = pty_elapsed_ms,
            parser_elapsed_ms = parser_elapsed_ms,
            total_elapsed_ms = total_elapsed_ms,
            "pane resize completed"
        );
        Ok(())
    }

    /// Kill the child process.
    pub fn kill(&self) -> Result<(), TerminalError> {
        self.pty.kill()
    }

    /// Get a reader for the PTY output.
    pub fn reader(&self) -> Result<Box<dyn std::io::Read + Send>, TerminalError> {
        self.pty.reader()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::test_util::{
        answer_cursor_position_query, echo_command, lock_pty_test, read_until_contains,
        read_with_timeout, sleep_command, stdin_echo_command, success_command, TestCommand,
    };

    fn test_pane(id: &str, command: TestCommand) -> Pane {
        Pane::new(
            id.to_string(),
            command.command,
            command.args,
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed")
    }

    fn test_pane_with_rows(id: &str, rows: u16, command: TestCommand) -> Pane {
        Pane::new(
            id.to_string(),
            command.command,
            command.args,
            80,
            rows,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed")
    }

    #[test]
    fn test_pane_creation() {
        let _pty_guard = lock_pty_test();
        let pane = test_pane("test-1", echo_command("hello"));

        assert_eq!(pane.id(), "test-1");
        assert_eq!(pane.status(), &PaneStatus::Running);
        assert_eq!(pane.scrollback_len(), 0);
    }

    #[test]
    fn test_process_bytes_updates_screen() {
        let _pty_guard = lock_pty_test();
        let mut pane = test_pane("test-2", sleep_command("60"));

        // Feed some bytes through the vt100 parser
        pane.process_bytes(b"hello world\r\n");

        let screen = pane.screen();
        let contents = screen.contents();
        assert!(
            contents.contains("hello world"),
            "Screen should contain 'hello world', got: {contents}"
        );

        let _ = pane.kill();
    }

    fn replay_snapshot(source: &vt100::Parser, max_scrollback: usize) -> vt100::Parser {
        let (rows, cols) = source.screen().size();
        let mut replay = vt100::Parser::new(rows, cols, max_scrollback);
        replay.process(&source.screen().snapshot_formatted(max_scrollback));
        replay
    }

    fn screen_at_oldest_scrollback(screen: &vt100::Screen) -> vt100::Screen {
        let mut screen = screen.clone();
        screen.set_scrollback(usize::MAX);
        screen
    }

    #[test]
    fn test_semantic_snapshot_round_trips_cursor_movement_and_erase() {
        let mut source = vt100::Parser::new(4, 12, 32);
        source.process(b"alpha\r\nbravo\r\ncharlie");
        source.process(b"\x1b[2;3H\x1b[4XOK\x1b[1;1H\x1b[K\x1b[3;5H");
        source.process(b"\x1b]2;must-not-replay\x07");

        let snapshot = source.screen().snapshot_formatted(32);
        assert!(
            !snapshot.windows(2).any(|window| window == b"\x1b]"),
            "semantic snapshots must not replay OSC side effects"
        );

        let replay = replay_snapshot(&source, 32);
        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
        assert_eq!(
            replay.screen().contents_formatted(),
            source.screen().contents_formatted()
        );

        let mut pending_source = vt100::Parser::new(3, 4, 8);
        pending_source.process(b"abcd");
        let mut pending_replay = replay_snapshot(&pending_source, 8);
        pending_source.process(b"X");
        pending_replay.process(b"X");
        assert_eq!(
            pending_replay.screen().contents(),
            pending_source.screen().contents(),
            "a current pending-wrap cursor must wrap the next output after replay"
        );
        assert_eq!(
            pending_replay.screen().cursor_position(),
            pending_source.screen().cursor_position()
        );
    }

    #[test]
    fn test_semantic_snapshot_round_trips_styled_history_without_visible_overlap() {
        let mut source = vt100::Parser::new(4, 12, 32);
        for line in 1..=10 {
            source.process(format!("\x1b[3{}mline-{line:02}\x1b[0m\r\n", line % 7 + 1).as_bytes());
        }

        let replay = replay_snapshot(&source, 32);
        let source_oldest = screen_at_oldest_scrollback(source.screen());
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());

        assert_eq!(replay_oldest.scrollback(), source_oldest.scrollback());
        assert_eq!(replay_oldest.contents(), source_oldest.contents());
        assert_eq!(
            replay_oldest.contents_formatted(),
            source_oldest.contents_formatted(),
            "history styling and the history-visible boundary must round-trip exactly"
        );

        let limited_replay = replay_snapshot(&source, 3);
        let limited_oldest = screen_at_oldest_scrollback(limited_replay.screen());
        let mut expected_limited = source.screen().clone();
        expected_limited.set_scrollback(3);
        assert_eq!(limited_oldest.scrollback(), 3);
        assert_eq!(
            limited_oldest.contents_formatted(),
            expected_limited.contents_formatted(),
            "snapshot history must honor the requested parsed-scrollback bound"
        );
    }

    #[test]
    fn test_semantic_snapshot_round_trips_soft_wrap_and_wide_characters() {
        let mut source = vt100::Parser::new(3, 6, 32);
        source.process("ab漢cdEFgh漢ijKLmn".as_bytes());

        let replay = replay_snapshot(&source, 32);
        let source_oldest = screen_at_oldest_scrollback(source.screen());
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());

        assert_eq!(replay_oldest.scrollback(), source_oldest.scrollback());
        assert_eq!(replay_oldest.contents(), source_oldest.contents());
        for row in 0..source.screen().size().0 {
            assert_eq!(
                replay_oldest.row_wrapped(row),
                source_oldest.row_wrapped(row),
                "soft-wrap state differs at row {row}"
            );
        }
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
    }

    #[test]
    fn test_semantic_snapshot_preserves_full_scrollback_row_after_narrow_resize() {
        let mut source = vt100::Parser::new(3, 12, 32);
        source.process(b"ABCDEFGHIJ\r\ntwo\r\nthree\r\nfour");
        resize_parser_preserving_state(&mut source, 3, 6);

        let replay = replay_snapshot(&source, 32);
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());

        for (col, expected) in ["A", "B", "C", "D", "E", "F"].iter().enumerate() {
            assert_eq!(
                replay_oldest
                    .cell(0, col.try_into().unwrap())
                    .expect("first reflowed history row cell")
                    .contents(),
                *expected
            );
        }
        for (col, expected) in ["G", "H", "I", "J"].iter().enumerate() {
            assert_eq!(
                replay_oldest
                    .cell(1, col.try_into().unwrap())
                    .expect("second reflowed history row cell")
                    .contents(),
                *expected,
                "old-width history suffix was lost at reflowed column {col}"
            );
        }
    }

    #[test]
    fn test_semantic_snapshot_preserves_scrollback_sgr_after_narrow_resize() {
        let mut source = vt100::Parser::new(3, 12, 32);
        source.process(b"\x1b[31mABCDEF\x1b[34mGHIJ\x1b[0m\r\ntwo\r\nthree\r\nfour");
        resize_parser_preserving_state(&mut source, 3, 6);

        let replay = replay_snapshot(&source, 32);
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());

        for col in 0..6 {
            assert_eq!(
                replay_oldest
                    .cell(0, col)
                    .expect("red history cell")
                    .fgcolor(),
                vt100::Color::Idx(1),
                "red SGR attribute changed at first reflowed row column {col}"
            );
        }
        for col in 0..4 {
            assert_eq!(
                replay_oldest
                    .cell(1, col)
                    .expect("blue history cell")
                    .fgcolor(),
                vt100::Color::Idx(4),
                "blue SGR attribute changed at second reflowed row column {col}"
            );
        }
    }

    #[test]
    fn test_semantic_snapshot_does_not_split_wide_glyph_at_reflow_boundary() {
        let mut source = vt100::Parser::new(3, 12, 32);
        source.process("abcde漢Z\r\ntwo\r\nthree\r\nfour".as_bytes());
        resize_parser_preserving_state(&mut source, 3, 6);

        let replay = replay_snapshot(&source, 32);
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());

        assert_eq!(
            replay_oldest
                .cell(1, 0)
                .expect("wide history cell")
                .contents(),
            "漢"
        );
        assert!(
            replay_oldest
                .cell(1, 1)
                .expect("wide history continuation")
                .is_wide_continuation(),
            "wide glyph continuation must remain adjacent after reflow"
        );
        assert_eq!(
            replay_oldest
                .cell(1, 2)
                .expect("cell after wide history glyph")
                .contents(),
            "Z",
            "cell following a boundary-wide glyph must not be lost"
        );
    }

    #[test]
    fn test_semantic_snapshot_applies_history_limit_after_narrow_reflow() {
        let mut source = vt100::Parser::new(3, 12, 32);
        source.process(b"ABCDEFGHIJ\r\nKLMNOPQRST\r\nthree\r\nfour\r\nfive");
        resize_parser_preserving_state(&mut source, 3, 6);

        let replay = replay_snapshot(&source, 3);
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());

        assert_eq!(
            replay_oldest.scrollback(),
            3,
            "max_scrollback must bound reflowed physical history rows"
        );
        assert_eq!(
            replay_oldest
                .cell(0, 0)
                .expect("oldest retained physical history row")
                .contents(),
            "G",
            "the newest physical history rows must be retained after reflow"
        );
        assert_eq!(
            replay_oldest
                .cell(1, 0)
                .expect("first physical row from newest logical line")
                .contents(),
            "K"
        );
        assert_eq!(
            replay_oldest
                .cell(2, 0)
                .expect("second physical row from newest logical line")
                .contents(),
            "Q"
        );
    }

    #[test]
    fn test_semantic_snapshot_replays_wide_history_at_single_column() {
        let mut source = vt100::Parser::new(3, 4, 32);
        source.process("\x1b[31m漢\x1b[0mA\r\nx\r\ny\r\nz".as_bytes());
        resize_parser_preserving_state(&mut source, 3, 1);

        let snapshot = source.screen().snapshot_formatted(32);
        let replay = std::panic::catch_unwind(|| {
            let mut replay = vt100::Parser::new(3, 1, 32);
            replay.process(&snapshot);
            replay
        });

        assert!(
            replay.is_ok(),
            "a one-column snapshot must replay wide history without panicking"
        );
        let replay = replay.expect("one-column replay");
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());
        let wide_cell = replay_oldest.cell(0, 0).expect("single-column wide cell");
        assert_eq!(wide_cell.contents(), "漢");
        assert_eq!(wide_cell.fgcolor(), vt100::Color::Idx(1));
        assert!(
            !wide_cell.is_wide(),
            "a wide glyph must use one effective cell in a one-column terminal"
        );
        assert_eq!(
            replay_oldest
                .cell(1, 0)
                .expect("ASCII cell following wide history")
                .contents(),
            "A",
            "the ASCII cell following a collapsed wide glyph must be retained"
        );
    }

    #[test]
    fn test_semantic_snapshot_normalizes_zero_terminal_size() {
        let result = std::panic::catch_unwind(|| {
            let mut source = vt100::Parser::new(0, 0, 8);
            assert_eq!(source.screen().size(), (1, 1));
            source.process(b"A");
            source.screen_mut().set_size(0, 0);
            assert_eq!(source.screen().size(), (1, 1));

            let snapshot = source.screen().snapshot_formatted(8);
            let mut replay = vt100::Parser::new(0, 0, 8);
            replay.process(&snapshot);
            replay
        });

        assert!(
            result.is_ok(),
            "zero-sized parser input must normalize and snapshot without panicking or stalling"
        );
        assert_eq!(
            result
                .expect("normalized zero-sized replay")
                .screen()
                .size(),
            (1, 1)
        );
    }

    #[test]
    fn test_semantic_snapshot_round_trips_active_alternate_screen_and_restore_state() {
        let mut source = vt100::Parser::new(3, 10, 32);
        source.process(b"\x1b[?1h\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b=\x1b[?25l");
        source.process(b"\x1b[32mPRIMARY\r\n$ \x1b[2;5H");
        source.process(b"\x1b[?1049h\x1b[34mALT\x1b[2;3H@");

        let mut replay = replay_snapshot(&source, 32);
        assert!(replay.screen().alternate_screen());
        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
        assert_eq!(
            replay.screen().application_cursor(),
            source.screen().application_cursor()
        );
        assert_eq!(
            replay.screen().application_keypad(),
            source.screen().application_keypad()
        );
        assert_eq!(
            replay.screen().bracketed_paste(),
            source.screen().bracketed_paste()
        );
        assert_eq!(
            replay.screen().mouse_protocol_mode(),
            source.screen().mouse_protocol_mode()
        );
        assert_eq!(
            replay.screen().mouse_protocol_mode(),
            vt100::MouseProtocolMode::ButtonMotion
        );
        assert_eq!(
            replay.screen().mouse_protocol_encoding(),
            source.screen().mouse_protocol_encoding()
        );
        assert_eq!(
            replay.screen().mouse_protocol_encoding(),
            vt100::MouseProtocolEncoding::Sgr
        );
        assert_eq!(replay.screen().hide_cursor(), source.screen().hide_cursor());

        source.process(b"\x1b[?1049l");
        replay.process(b"\x1b[?1049l");
        assert!(!replay.screen().alternate_screen());
        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position(),
            "the saved primary cursor must survive alternate-screen replay"
        );
        assert_eq!(replay.screen().fgcolor(), source.screen().fgcolor());
    }

    #[test]
    fn test_semantic_snapshot_preserves_saved_cursor_and_attributes_for_continuation() {
        let mut source = vt100::Parser::new(4, 12, 32);
        source.process(b"\x1b[2;4H\x1b[31;1mS\x1b7");
        source.process(b"\x1b[4;9H\x1b[34mC");
        let mut replay = replay_snapshot(&source, 32);

        source.process(b"\x1b8R");
        replay.process(b"\x1b8R");

        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
        assert_eq!(replay.screen().fgcolor(), source.screen().fgcolor());
        assert_eq!(replay.screen().bold(), source.screen().bold());
        assert_eq!(
            replay.screen().cell(1, 4).expect("restored cell").fgcolor(),
            source.screen().cell(1, 4).expect("restored cell").fgcolor()
        );

        let mut pending_source = vt100::Parser::new(3, 4, 8);
        pending_source.process(b"abcd\x1b7\x1b[3;1HC");
        let mut pending_replay = replay_snapshot(&pending_source, 8);
        pending_source.process(b"\x1b8X");
        pending_replay.process(b"\x1b8X");
        assert_eq!(
            pending_replay.screen().contents(),
            pending_source.screen().contents(),
            "a saved pending-wrap cursor must wrap the next output after replay"
        );
        assert_eq!(
            pending_replay.screen().cursor_position(),
            pending_source.screen().cursor_position()
        );
        assert_eq!(
            pending_replay.screen().row_wrapped(0),
            pending_source.screen().row_wrapped(0)
        );
    }

    #[test]
    fn test_semantic_snapshot_preserves_reachable_blank_pending_cursor_continuation() {
        // T-165 / FR-003p: this is the reachable compound state identified by
        // review. A stricter assertion that ESC8 must also restore the distinct
        // outer ESC7 state was RED (exit 101): terminal byte streams have only
        // one active-buffer saved-cursor slot, so reconstructing a blank
        // pending-wrap cursor consumes that slot. Phase 16A therefore requires
        // exact current-frame and printable-continuation fidelity here, while
        // the representable saved-state contract remains covered above.
        let mut source = vt100::Parser::new(3, 4, 8);
        source.process(b"\x1b[1;2H\x1b[31m\x1b7\x1b[3;1H\x1b[34mABCD\x1b[S");

        assert_eq!(source.screen().cursor_position(), (2, 4));
        assert_eq!(source.screen().fgcolor(), vt100::Color::Idx(4));
        assert!(!source
            .screen()
            .cell(2, 3)
            .expect("blank pending cell")
            .has_contents());

        let mut replay = replay_snapshot(&source, 8);
        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
        assert_eq!(replay.screen().fgcolor(), source.screen().fgcolor());
        for row in 0..source.screen().size().0 {
            for col in 0..source.screen().size().1 {
                assert_eq!(
                    replay.screen().cell(row, col),
                    source.screen().cell(row, col),
                    "current-frame cell differs at ({row}, {col})"
                );
            }
        }
        assert!(!replay
            .screen()
            .cell(2, 3)
            .expect("replayed blank pending cell")
            .has_contents());

        source.process(b"X");
        replay.process(b"X");
        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
        for row in 0..source.screen().size().0 {
            assert_eq!(
                replay.screen().row_wrapped(row),
                source.screen().row_wrapped(row),
                "printable continuation wrap differs at row {row}"
            );
        }
        let source_oldest = screen_at_oldest_scrollback(source.screen());
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());
        assert_eq!(replay_oldest.scrollback(), source_oldest.scrollback());
        assert_eq!(
            replay_oldest.contents_formatted(),
            source_oldest.contents_formatted(),
            "printable continuation must preserve bounded history and cell attributes"
        );
    }

    #[test]
    fn test_semantic_snapshot_preserves_scroll_region_for_lf_continuation() {
        let mut source = vt100::Parser::new(5, 8, 32);
        source.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        source.process(b"\x1b[2;4r\x1b[4;1H");
        let mut replay = replay_snapshot(&source, 32);

        source.process(b"\nX");
        replay.process(b"\nX");

        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
    }

    #[test]
    fn test_semantic_snapshot_preserves_origin_mode_for_relative_cup_continuation() {
        let mut source = vt100::Parser::new(5, 8, 32);
        source.process(b"\x1b[2;4r\x1b[?6h\x1b[2;3HX");
        let mut replay = replay_snapshot(&source, 32);

        source.process(b"\x1b[1;1HZ");
        replay.process(b"\x1b[1;1HZ");

        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
        assert_eq!(
            replay
                .screen()
                .cell(1, 0)
                .expect("origin-relative cell")
                .contents(),
            "Z"
        );
    }

    #[test]
    fn test_semantic_snapshot_preserves_bounded_normal_history_and_alternate_continuation() {
        let mut source = vt100::Parser::new(4, 10, 32);
        for line in 1..=8 {
            source.process(format!("normal-{line}\r\n").as_bytes());
        }
        source.process(b"\x1b[?1049h");
        source.process(b"\x1b[2;3r\x1b[?6h\x1b[2;2H\x1b[35mS\x1b7");
        source.process(b"\x1b[1;4H\x1b[36mC");

        let snapshot = source.screen().snapshot_formatted(3);
        let mut replay = vt100::Parser::new(4, 10, 3);
        replay.process(&snapshot);

        source.process(b"\x1b8R\nN");
        replay.process(b"\x1b8R\nN");
        assert!(replay.screen().alternate_screen());
        assert_eq!(replay.screen().contents(), source.screen().contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
        assert_eq!(replay.screen().fgcolor(), source.screen().fgcolor());

        source.process(b"\x1b[?1049l");
        replay.process(b"\x1b[?1049l");
        let source_primary = screen_at_oldest_scrollback(source.screen());
        let replay_primary = screen_at_oldest_scrollback(replay.screen());
        assert_eq!(replay_primary.scrollback(), 3);
        let mut expected_primary = source_primary.clone();
        expected_primary.set_scrollback(3);
        assert_eq!(replay_primary.contents(), expected_primary.contents());
        assert_eq!(
            replay.screen().cursor_position(),
            source.screen().cursor_position()
        );
    }

    #[test]
    fn test_snapshot_bytes_preserves_boundary_scrollback_line() {
        let _pty_guard = lock_pty_test();
        let mut pane = test_pane_with_rows("test-boundary", 6, sleep_command("60"));

        for line in 1..=18 {
            pane.process_bytes(format!("line-{line:02}\r\n").as_bytes());
        }

        let snapshot = String::from_utf8_lossy(&pane.snapshot_bytes()).into_owned();

        assert!(
            snapshot.contains("line-13"),
            "snapshot should include the boundary scrollback line; got: {snapshot:?}"
        );
    }

    #[test]
    fn test_pane_snapshot_bounds_parsed_history_at_replay_limit() {
        let _pty_guard = lock_pty_test();
        let mut pane = test_pane_with_rows("test-parsed-history-bound", 3, sleep_command("60"));

        // Three rows plus 5,000 additional CRLF advances produce 5,001
        // historical rows. Parsed history must evict the first one at the
        // snapshot replay boundary, while the legacy raw-line store retains
        // all completed lines under its independent 10,000-line policy.
        for line in 1..=SNAPSHOT_SCROLLBACK_REPLAY_LIMIT + 3 {
            pane.process_bytes(format!("line-{line:04}\r\n").as_bytes());
        }

        let source_oldest = screen_at_oldest_scrollback(pane.screen());
        assert_eq!(
            source_oldest.scrollback(),
            SNAPSHOT_SCROLLBACK_REPLAY_LIMIT,
            "Pane parsed history must be bounded at the snapshot replay limit"
        );
        assert_eq!(
            pane.scrollback_len(),
            SNAPSHOT_SCROLLBACK_REPLAY_LIMIT + 3,
            "raw scrollback storage keeps its independent compatibility capacity"
        );

        let mut replay = vt100::Parser::new(3, 80, SNAPSHOT_SCROLLBACK_REPLAY_LIMIT);
        replay.process(&pane.snapshot_bytes());
        let replay_oldest = screen_at_oldest_scrollback(replay.screen());
        assert_eq!(replay_oldest.scrollback(), SNAPSHOT_SCROLLBACK_REPLAY_LIMIT);
        let oldest_contents = replay_oldest.contents();
        assert!(
            oldest_contents.contains("line-0002"),
            "snapshot must retain the oldest line inside the 5,000-line boundary: {oldest_contents:?}"
        );
        assert!(
            !oldest_contents.contains("line-0001"),
            "snapshot must not retain the line immediately before the boundary: {oldest_contents:?}"
        );

        let _ = pane.kill();
    }

    #[test]
    fn test_snapshot_bytes_uses_visible_rows_for_scrollback_overlap() {
        let _pty_guard = lock_pty_test();
        let mut pane = test_pane_with_rows("test-visible-overlap", 6, sleep_command("60"));

        for line in 1..=18 {
            pane.process_bytes(format!("line-{line:02}\r\n").as_bytes());
        }
        pane.process_bytes(b"\x1b[2;1H");

        let snapshot = String::from_utf8_lossy(&pane.snapshot_bytes()).into_owned();
        let repeated_visible_line = snapshot.matches("line-14").count();

        assert_eq!(
            repeated_visible_line, 1,
            "snapshot should not replay visible scrollback lines when cursor moves; got: {snapshot:?}"
        );
    }

    #[test]
    fn test_pane_read_output_through_vt100() {
        let _pty_guard = lock_pty_test();
        let pane = test_pane("test-3", echo_command("vt100-test"));
        answer_cursor_position_query(pane.pty());

        let reader = pane.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("vt100-test"),
            "Expected 'vt100-test' in: {text}"
        );
    }

    #[test]
    fn test_pane_write_input() {
        let _pty_guard = lock_pty_test();
        let pane = test_pane("test-4", stdin_echo_command());
        answer_cursor_position_query(pane.pty());

        let reader = pane.reader().expect("reader failed");
        pane.write_input(b"pane-input\n").expect("write failed");
        let output =
            read_until_contains(reader, Duration::from_secs(5), "pane-input").expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("pane-input"),
            "Expected 'pane-input' in: {text}"
        );
    }

    #[test]
    fn test_pane_resize() {
        let _pty_guard = lock_pty_test();
        let mut pane = test_pane("test-5", sleep_command("60"));

        pane.resize(120, 48).expect("resize should succeed");

        // vt100 parser should reflect new size
        let screen = pane.screen();
        assert_eq!(screen.size(), (48, 120));

        let _ = pane.kill();
    }

    #[test]
    fn test_resize_parser_handles_wide_char_shrink_without_followup_panic() {
        let mut parser = vt100::Parser::new(1, 4, 0);
        parser.process("ab漢".as_bytes());

        resize_parser_preserving_state(&mut parser, 1, 3);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parser.process(b"\x1b[1;3H\x1b[K");
            parser.screen().contents()
        }));

        assert!(
            result.is_ok(),
            "shrinking after a trailing wide glyph must not panic on follow-up erase"
        );
        assert_eq!(parser.screen().size(), (1, 3));
    }

    #[test]
    fn test_resize_parser_drops_truncated_wide_glyph_from_snapshot() {
        let mut parser = vt100::Parser::new(2, 4, 0);
        parser.process("ab漢".as_bytes());

        resize_parser_preserving_state(&mut parser, 2, 3);

        let snapshot = parser.screen().contents();
        assert!(
            snapshot.starts_with("ab"),
            "snapshot should preserve visible prefix"
        );
        assert!(
            !snapshot.contains('漢'),
            "snapshot must drop a wide glyph that no longer fits in the narrower width"
        );
    }

    #[test]
    fn test_resize_parser_handles_release_panic_width_boundary() {
        let mut parser = vt100::Parser::new(1, 83, 0);
        let line = format!("{}漢", "a".repeat(81));
        parser.process(line.as_bytes());

        resize_parser_preserving_state(&mut parser, 1, 82);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parser.process(b"\x1b[1;82H\x1b[K");
            parser.screen().contents()
        }));

        assert!(
            result.is_ok(),
            "shrinking to 82 columns must not leave a wide glyph at index 81"
        );
        assert_eq!(parser.screen().size(), (1, 82));
    }

    #[test]
    fn test_resize_parser_preserves_alternate_screen_restore_state() {
        let mut parser = vt100::Parser::new(2, 4, 0);
        parser.process(b"sh\r\n$ ");
        assert_eq!(parser.screen().cursor_position(), (1, 2));

        parser.process(b"\x1b[?1049h");
        assert!(parser.screen().alternate_screen());
        parser.process("ab漢".as_bytes());

        resize_parser_preserving_state(&mut parser, 2, 3);

        assert!(
            parser.screen().alternate_screen(),
            "narrow resize must keep alternate-screen mode active until ?1049l"
        );
        parser.process(b"\x1b[?1049l");

        assert!(
            !parser.screen().alternate_screen(),
            "alternate-screen mode must clear only after ?1049l"
        );
        assert!(
            parser.screen().contents().contains("sh"),
            "restored primary grid should still contain the shell buffer"
        );
        assert_eq!(
            parser.screen().cursor_position(),
            (1, 2),
            "saved primary cursor must survive alternate-screen resize"
        );
    }

    #[test]
    fn test_resize_parser_preserves_row_attributes_when_truncating_wide_glyph() {
        let mut parser = vt100::Parser::new(1, 4, 0);
        parser.process("\x1b[31;44mab漢".as_bytes());

        resize_parser_preserving_state(&mut parser, 1, 3);

        let first = parser.screen().cell(0, 0).expect("cell 0");
        let second = parser.screen().cell(0, 1).expect("cell 1");
        let trailing = parser.screen().cell(0, 2).expect("cell 2");

        assert_eq!(first.contents(), "a");
        assert_eq!(second.contents(), "b");
        assert!(
            !trailing.has_contents(),
            "truncated wide glyph must be cleared"
        );

        for cell in [first, second, trailing] {
            assert_eq!(cell.fgcolor(), vt100::Color::Idx(1));
            assert_eq!(cell.bgcolor(), vt100::Color::Idx(4));
        }
    }

    #[test]
    fn test_pane_check_status_completed() {
        let _pty_guard = lock_pty_test();
        let mut pane = test_pane("test-6", success_command());
        answer_cursor_position_query(pane.pty());

        assert_eq!(pane.status(), &PaneStatus::Running);

        let mut completed = false;
        for _ in 0..50 {
            if let Ok(status) = pane.check_status() {
                if *status != PaneStatus::Running {
                    completed = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(completed, "Process should have completed");
        assert_eq!(pane.status(), &PaneStatus::Completed(0));
    }

    #[test]
    fn test_pane_mark_error() {
        let _pty_guard = lock_pty_test();
        let mut pane = test_pane("test-7", sleep_command("60"));

        pane.mark_error("test error");
        assert_eq!(pane.status(), &PaneStatus::Error("test error".to_string()));

        let _ = pane.kill();
    }

    #[test]
    fn test_pane_kill() {
        let _pty_guard = lock_pty_test();
        let pane = test_pane("test-8", sleep_command("60"));

        pane.kill().expect("kill should succeed");
    }

    #[test]
    fn test_pane_status_enum() {
        let running = PaneStatus::Running;
        let completed = PaneStatus::Completed(0);
        let error = PaneStatus::Error("fail".to_string());

        assert_eq!(running, PaneStatus::Running);
        assert_eq!(completed, PaneStatus::Completed(0));
        assert_ne!(PaneStatus::Completed(0), PaneStatus::Completed(1));
        assert_eq!(error, PaneStatus::Error("fail".to_string()));
    }
}
