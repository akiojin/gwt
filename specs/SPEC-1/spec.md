# Terminal Emulation -- vt100 Rendering, Scrollback, Selection

## Background

`renderer.rs` and related tests cover low-level vt100 cell rendering, URL underline detection, and alt-screen verification, but the current `gwt-tui` session pane still lacks the remaining interaction layer needed for smooth scrollback review, range selection, and scrollbar visibility. This SPEC therefore covers both the renderer-level work already in place and the still-missing session-surface behavior needed for URL opening, scrollback interaction, selection, and scrollbar rendering.

## User Stories

### US-1: View Agent Output with Full ANSI Color and Attribute Support (P0) -- PARTIALLY IMPLEMENTED

As a developer, I want agent output to render with full ANSI color and text attributes so that I can read formatted output naturally.

**Acceptance Scenarios**

1. Given PTY output containing ANSI 256-color escape sequences, when rendered in gwt-tui, then all Named, Indexed, and RGB colors display correctly.
2. Given PTY output containing bold, italic, underline, and strikethrough attributes, when rendered, then each attribute is visually distinct.
3. Given rapid PTY output (1000+ lines/sec), when rendering, then no frames are dropped and all content is eventually visible.

### US-2: Scroll Through Terminal History (P0) -- NOT IMPLEMENTED

As a developer, I want to scroll through terminal history so that I can review past output.

**Acceptance Scenarios**

1. Given a session with more than one screen of output, when I scroll up with mouse wheel, then earlier output becomes visible.
2. Given a scrollback buffer at maximum capacity (10,000 lines), when new output arrives, then the oldest lines are evicted first.
3. Given I have scrolled up, when new output arrives, then the viewport stays at my scroll position (no auto-jump).
4. Given the session has more history than fits on screen, when the terminal pane renders, then a vertical scrollbar appears on the right edge and its thumb tracks the visible scroll position.
5. Given the host terminal is Terminal.app, when gwt enters the alternate screen and the user scrolls over the session pane with a trackpad, then the gesture reaches gwt as scroll input rather than being translated into cursor keys by the host terminal.
6. Given Terminal.app reports a two-finger gesture as right-button drag events instead of wheel events, when the drag moves vertically over the session pane, then gwt maps that movement into scrollback navigation.
7. Given a pane redraws a full-screen UI without accumulating vt100 row scrollback, when multiple frames arrive and I scroll up, then recent earlier frames become visible from gwt's pane-local in-memory snapshot cache.
8. Given I am viewing an older in-memory snapshot, when new output arrives, then the viewport stays on that older frame until I scroll back to the newest frame.
9. Given Terminal.app leaks an SGR mouse report instead of a parsed mouse event, when that sequence reaches gwt, then it is consumed as mouse input and never rendered into the session pane as literal `[<...M` text.
10. Given the host terminal emits a burst of consecutive wheel events for one trackpad gesture, when the burst arrives over the session pane, then gwt applies the whole burst before the next redraw so scrolling stays responsive and boundary non-scroll input is preserved.
11. Given a pane is using snapshot-backed scrollback, when the scrollbar renders, then the thumb length reflects the visible terminal viewport height instead of collapsing to a single-cell frame indicator.

### US-3: Select and Copy Text from Terminal Output (P1) -- NOT IMPLEMENTED

As a developer, I want to select text with mouse drag and copy it so that I can paste terminal output elsewhere.

**Acceptance Scenarios**

1. Given terminal output is visible, when I click and drag the mouse, then the selected region is highlighted with reversed video.
2. Given a text selection exists, when I release the mouse button, then the selected text is copied to the system clipboard.
3. Given a selection spans multiple lines, when copied, then line breaks are preserved correctly.

### US-4: Click URLs in Terminal Output to Open in Browser (P1) -- NOT IMPLEMENTED

As a developer, I want to click URLs in terminal output so that I can quickly open links without manual copy-paste.

**Acceptance Scenarios**

1. Given terminal output contains a URL (http:// or https://), when rendered, then the URL text is underlined.
2. Given a detected URL is visible, when I Ctrl+click it, then my default browser opens the URL.
3. Given terminal output contains multiple URLs on the same line, when rendered, then each URL is independently clickable.
4. Given a URL wraps across two terminal lines, when detected, then the full URL is recognized and clickable.

### US-5: Run TUI Apps That Use Alt-Screen Buffer (P2) -- VERIFIED AT RENDERER LAYER ONLY

As a developer, I want TUI applications (vi, top, htop) running inside gwt sessions to display correctly using the alternate screen buffer.

**Acceptance Scenarios**

1. Given a TUI app (e.g., vi) is launched inside a gwt session, when it enters alt-screen mode, then the main scrollback is preserved and the alt-screen content renders.
2. Given a TUI app exits alt-screen mode, when the main screen restores, then the original scrollback content is intact.
3. Given a TUI app uses cursor movement and screen clearing, when rendering, then the display matches native terminal behavior.

## Edge Cases

- PTY output contains incomplete/malformed ANSI escape sequences mid-stream.
- Extremely long lines (>10,000 characters) that exceed terminal width.
- Binary data accidentally written to PTY (non-UTF-8 bytes).
- Scrollback buffer boundary: exactly 10,000 lines with rapid new output.
- URL containing special characters (parentheses, query strings, fragments).
- URL at the very end of scrollback buffer about to be evicted.
- Mouse selection across a region containing wide (CJK) characters.
- Scrollbar gutter on narrow terminals should not corrupt wrapped text layout or cursor placement.
- Selection starting in visible history and ending after additional scroll movement should still copy the intended region.
- Alt-screen app sends output after gwt session is backgrounded.
- Pane closes while snapshot history exists; reopening the pane should start from live output only.

## Functional Requirements

- **FR-001**: `vt100::Parser` processes raw PTY bytes into a screen buffer with cell-level color and attribute data.
- **FR-002**: `renderer.rs` converts vt100 cells to ratatui `Buffer` with color mapping: Named to Named, Indexed to Indexed, RGB to Rgb.
- **FR-003**: Scrollback buffer stores up to 10,000 lines per pane, configurable via settings.
- **FR-003a**: When a pane's visible screen does not expose vt100 row scrollback, gwt keeps a pane-local in-memory ring buffer of recent distinct screen snapshots for the lifetime of that pane.
- **FR-003b**: Snapshot scrollback is ephemeral only: gwt does not preload Codex / Claude transcript files for this feature, and the cache is discarded when the pane closes.
- **FR-004**: Mouse wheel and trackpad scrolling is always active when the terminal pane has focus.
- **FR-004b**: On startup gwt disables host-terminal alternate-scroll mode for its alternate-screen session so Terminal.app trackpad gestures reach gwt's mouse scroll handling.
- **FR-004c**: When Terminal.app reports trackpad motion as `Down/Drag/Up(Right)` over the session pane, gwt interprets the vertical drag delta as scrollback motion without affecting left-button text selection.
- **FR-004a**: A vertical scrollbar is rendered on the right edge only when row scrollback or snapshot history exceeds the visible terminal height / frame count.
- **FR-004d**: If the outer terminal leaks an SGR mouse report as an escape-key sequence, gwt normalizes it back into mouse input (or swallows it) before PTY forwarding so literal mouse-report text is never echoed inside the pane.
- **FR-004e**: Consecutive wheel events that are already waiting in the outer-terminal queue are drained as a bounded burst before the next render pass so one gesture does not force one full redraw per raw wheel event.
- **FR-005**: Live-follow mode auto-scrolls to the bottom on new output; disengages when user scrolls up.
- **FR-005a**: The scrollbar thumb position and size are derived from the current viewport height and scrollback position so the indicator matches the visible slice.
- **FR-005b**: While the user is viewing an older snapshot-backed frame, new output appends to the history cache without forcing the viewport back to live until the user scrolls down to the newest frame.
- **FR-005c**: Snapshot-backed scrollbar metrics use the visible viewport height plus the number of extra historical frames so the thumb length stays proportional to the pane instead of shrinking to a single cell.
- **FR-006**: Text selection via mouse drag with reversed-video highlight on selected cells.
- **FR-006a**: Selection coordinates are tracked in viewport cell space and resolved against the active scrollback offset so copied text matches the currently visible history.
- **FR-007**: Copy selected text to system clipboard via platform-native clipboard integration.
- **FR-008**: URL detection in terminal output using regex pattern matching (http/https schemes).
- **FR-009**: Ctrl+click or Enter on a detected URL opens the default browser via `open` (macOS) / `xdg-open` (Linux).
- **FR-010**: Alt-screen buffer support: vt100 crate handles DECSET 1049 for alternate screen activation/deactivation.
- **FR-011**: Rendering handles wide characters (CJK) with correct 2-cell width accounting.
- **FR-012**: Malformed ANSI sequences are silently discarded without corrupting the screen buffer.

## Non-Functional Requirements

- **NFR-001**: Rendering latency under 16ms per frame to maintain 60fps visual smoothness.
- **NFR-002**: Memory usage proportional to scrollback size; 10,000 lines should consume under 50MB per pane.
- **NFR-002a**: Snapshot-backed scrollback uses a fixed-size in-memory ring buffer (256 frames) so full-screen panes remain bounded without transcript persistence.
- **NFR-003**: Cross-platform support via crossterm backend (macOS, Linux, Windows).
- **NFR-004**: No visible flicker during rapid output (smooth rendering pipeline).
- **NFR-004a**: Hover-only mouse-move floods do not trigger session redraw work because gwt does not use pointer-move hover semantics in the terminal pane.
- **NFR-004b**: High-frequency wheel bursts from host terminals such as Terminal.app do not degrade interaction by forcing a full frame render for every raw wheel event in the burst.
- **NFR-005**: Clipboard operations complete within 100ms.
- **NFR-006**: URL detection adds no measurable latency to normal rendering path.

## Success Criteria

- **SC-001**: All 17+ existing keybind and viewport tests continue to pass.
- **SC-002**: URL detection correctly identifies URLs in test fixtures covering common patterns.
- **SC-003**: Ctrl+click on a URL invokes the platform browser opener with the correct URL.
- **SC-004**: Alt-screen buffer activation/deactivation preserves main scrollback integrity.
- **SC-005**: Color mapping tests cover Named, Indexed, and RGB color spaces.
- **SC-006**: Scrollback eviction at 10,000-line boundary works without data corruption.
- **SC-007**: Scrollbar chrome appears only for overflowing history or snapshot caches and the thumb position changes when the user scrolls.
- **SC-008**: Drag selection across single-line and multi-line scrollback copies the expected plain-text payload to the clipboard.
- **SC-009**: A full-screen pane with `max_scrollback == 0` still exposes recent frames through in-memory snapshot scrollback, and live-follow resumes only after the user returns to the newest frame.
- **SC-010**: Consecutive wheel events are batched before redraw, preserving the first non-scroll message after the burst so trackpad scrolling remains responsive under Terminal.app event floods.
- **SC-011**: Snapshot-backed scrollbars keep a viewport-sized thumb baseline, so short frame histories render a legible scrollbar length instead of a one-cell marker.
