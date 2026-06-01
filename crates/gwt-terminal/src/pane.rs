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
        let parser = vt100::Parser::new(rows, cols, 0);
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
    /// `vt100::Screen::contents_formatted()` only describes the currently
    /// visible grid. Prepending completed scrollback lines lets a fresh xterm.js
    /// instance rebuild normal-buffer history before the current screen is
    /// redrawn.
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut snapshot = Vec::new();
        let scrollback_len = self.scrollback.len();
        let visible_rows = usize::from(self.parser.screen().size().0);
        let replayable_len = scrollback_len.saturating_sub(visible_rows);
        let start = replayable_len.saturating_sub(SNAPSHOT_SCROLLBACK_REPLAY_LIMIT);

        for line in self.scrollback.get_lines(start, replayable_len - start) {
            append_snapshot_scrollback_line(&mut snapshot, line);
        }

        snapshot.extend_from_slice(&self.parser.screen().contents_formatted());
        snapshot
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

fn append_snapshot_scrollback_line(snapshot: &mut Vec<u8>, line: &ScrollbackLine) {
    let before = snapshot.len();
    let raw = if line.formatted.is_empty() {
        line.text.as_bytes()
    } else {
        line.formatted.as_slice()
    };

    append_sanitized_snapshot_line(snapshot, raw);
    if snapshot.len() > before {
        snapshot.push(b'\n');
    }
}

fn append_sanitized_snapshot_line(output: &mut Vec<u8>, raw: &[u8]) {
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'\x1b' => {
                index = append_or_skip_escape_sequence(output, raw, index);
            }
            b'\r' | b'\t' => {
                output.push(raw[index]);
                index += 1;
            }
            0x20..=0x7e | 0x80..=0xff => {
                output.push(raw[index]);
                index += 1;
            }
            _ => {
                index += 1;
            }
        }
    }
}

fn append_or_skip_escape_sequence(output: &mut Vec<u8>, raw: &[u8], start: usize) -> usize {
    let Some(kind) = raw.get(start + 1).copied() else {
        return start + 1;
    };
    match kind {
        b'[' => append_or_skip_csi_sequence(output, raw, start),
        b']' => skip_osc_sequence(raw, start),
        _ => (start + 2).min(raw.len()),
    }
}

fn append_or_skip_csi_sequence(output: &mut Vec<u8>, raw: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index < raw.len() {
        let byte = raw[index];
        if (0x40..=0x7e).contains(&byte) {
            let next = index + 1;
            if byte == b'm' {
                output.extend_from_slice(&raw[start..next]);
            }
            return next;
        }
        index += 1;
    }
    raw.len()
}

fn skip_osc_sequence(raw: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index < raw.len() {
        if raw[index] == b'\x07' {
            return index + 1;
        }
        if raw[index] == b'\x1b' && raw.get(index + 1) == Some(&b'\\') {
            return index + 2;
        }
        index += 1;
    }
    raw.len()
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

    #[test]
    fn test_snapshot_sanitizer_preserves_sgr_but_drops_destructive_csi() {
        let mut output = Vec::new();

        append_sanitized_snapshot_line(&mut output, b"\x1b[H\x1b[J\x1b[31;1mALERT\x1b[0m\x1b[2K");

        assert_eq!(
            String::from_utf8_lossy(&output),
            "\x1b[31;1mALERT\x1b[0m",
            "scrollback replay must keep styling but not cursor moves or clears"
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
