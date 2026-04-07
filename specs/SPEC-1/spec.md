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
7. Given a pane redraws a full-screen UI without accumulating vt100 row scrollback, when the visible viewport advances vertically across multiple frames and I scroll up, then recent earlier viewports become visible from gwt's pane-local in-memory cache.
8. Given I am viewing an older in-memory snapshot, when new output arrives, then the viewport stays on that older frame until I scroll back to the newest frame.
9. Given Terminal.app leaks an SGR mouse report instead of a parsed mouse event, when that sequence reaches gwt, then it is consumed as mouse input and never rendered into the session pane as literal `[<...M` text.
10. Given the host terminal emits a burst of consecutive wheel events for one trackpad gesture, when the burst arrives over the session pane, then gwt applies the whole burst before the next redraw so scrolling stays responsive and boundary non-scroll input is preserved.
11. Given a pane is using snapshot-backed scrollback, when the scrollbar renders, then the thumb length reflects the visible terminal viewport height instead of collapsing to a single-cell frame indicator.
12. Given a full-screen pane redraw arrives as multiple PTY reader chunks inside one event-loop drain, when gwt records snapshot-backed scrollback, then it keeps only the final drained frame for that pass instead of exposing partially painted intermediate states during scrollback review.
13. Given a full-screen pane redraw overwrites or clears the same visible rows without advancing the viewport, when gwt updates its in-memory cache, then the new VT-interpreted frame is preserved as distinct history (when different) and stale cleared lines are not shown in the visible frame.
14. Given the previous full-screen frame is visually blank and the next frame only introduces content near the bottom rows, when gwt updates snapshot history, then leading blank history is pruned so scrolling to the oldest frame never shows an empty phantom screen.
15. Given historical snapshots already include an old blank frame and newer frames contain visible text, when gwt updates the snapshot history, then it prunes the leading blank frame so scrolling to the oldest position still shows meaningful content.
16. Given snapshot-backed scrollback is at live-follow and I scroll one step upward, when gwt enters history mode, then it lands on the immediately previous snapshot (one-step movement) instead of skipping older frames.
17. Given a leaked SGR wheel sequence arrives with small per-character delays, when gwt normalizes that input, then the entire sequence is consumed as mouse input and never appears as literal `[<...M` text.
18. Given Terminal pane is not focused and host leaks SGR wheel sequences, when the user scrolls over the session area, then gwt still normalizes the sequence into mouse events and can focus+scroll the session instead of forwarding literal escape fragments.
19. Given full-screen redraws mutate overlap rows (for example header/status churn), when the resulting visible VT frame differs from the previous frame, then gwt records that frame as a new snapshot without relying on overlap heuristics.
20. Given a pane has existing main-screen row scrollback and then enters alternate screen, when the user scrolls while viewing alternate-screen output, then gwt uses snapshot-backed history for the visible alternate-screen frames instead of stale main-screen row scrollback.
21. Given scrollback interaction, URL hit-testing, selection copy, and terminal rendering happen in one pane, when the viewport moves, then all of them resolve against the same cached visible surface instead of mixing separate live/snapshot sources.
22. Given a Claude/Codex agent pane is running and has more than one screen of PTY output, when the user scrolls upward through recent history, then gwt reads only the VT-derived in-memory cache and does not switch to session-log-derived text.
23. Given a Claude/Codex agent pane outputs ANSI-styled text, when the user scrolls through its in-memory history, then colors and text attributes remain visible throughout the reachable cache.
24. Given a Claude/Codex agent pane redraws launch, blank, or status frames in-place, when vt100 row scrollback exists, then gwt prefers the terminal-like row scrollback cache so overwritten transient screens do not appear as separate history entries.
25. Given a Claude/Codex agent pane redraws full-screen frames in-place and vt100 row scrollback stays at zero, when the user scrolls recent history, then gwt falls back to the same pane-local in-memory frame cache instead of losing scrollback entirely.
26. Given a Claude/Codex agent pane closes or gwt restarts, when the pane is opened again, then prior scrollback is not restored from session logs and history starts from fresh PTY output.
27. Given I am viewing older terminal history, when I press any key that is forwarded to the PTY, then gwt returns the viewport to live before sending that input.

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
- **FR-003a**: When a pane's visible screen does not expose vt100 row scrollback, gwt keeps a pane-local in-memory ring buffer of recent visible viewport states for the lifetime of that pane.
- **FR-003b**: Agent-pane scrollback cache remains ephemeral in memory and is discarded when the pane closes.
- **FR-003c**: For Claude/Codex agent panes, gwt does not hydrate runtime scrollback from session `jsonl` or session-log files; the only scrollback source is live PTY-derived in-memory cache.
- **FR-003d**: Agent-pane scrollback preserves VT-derived color and text attributes throughout the reachable in-memory history.
- **FR-003e**: For Claude/Codex agent panes, gwt prefers recent scrollback from a normalized row-based terminal buffer, so clear/redraw/launch screens that overwrite the viewport do not surface as separate scrollback entries when row history exists.
- **FR-003f**: When a Claude/Codex agent pane redraws full-screen content without advancing vt100 row scrollback, gwt falls back to the same pane-local in-memory snapshot history instead of session logs or transcript reconstruction.
- **FR-003g**: Agent-pane row scrollback capacity is larger than the standard terminal row-history limit while remaining ephemeral in memory for the lifetime of that pane.
- **FR-003h**: When an agent pane closes or gwt restarts, prior scrollback is discarded and is not reconstructed from persisted session artifacts.
- **FR-004**: Mouse wheel and trackpad scrolling is always active when the terminal pane has focus.
- **FR-004b**: On startup gwt disables host-terminal alternate-scroll mode for its alternate-screen session so Terminal.app trackpad gestures reach gwt's mouse scroll handling.
- **FR-004c**: When Terminal.app reports trackpad motion as `Down/Drag/Up(Right)` over the session pane, gwt interprets the vertical drag delta as scrollback motion without affecting left-button text selection.
- **FR-004a**: A vertical scrollbar is rendered on the right edge only when row scrollback or snapshot history exceeds the visible terminal height / frame count.
- **FR-004d**: If the outer terminal leaks an SGR mouse report as an escape-key sequence, gwt normalizes it back into mouse input (or swallows it) before PTY forwarding so literal mouse-report text is never echoed inside the pane.
- **FR-004e**: Consecutive wheel events that are already waiting in the outer-terminal queue are drained as a bounded burst before the next render pass so one gesture does not force one full redraw per raw wheel event.
- **FR-004f**: SGR mouse leak normalization uses inter-character inactivity timeout semantics so moderately delayed sequence fragments are still reconstructed as one mouse event instead of leaking partial literal text.
- **FR-004g**: SGR leak normalization is applied independent of current terminal-focus state so leaked wheel reports can still recover into mouse events that trigger session focus handoff and scrolling.
- **FR-005**: Live-follow mode auto-scrolls to the bottom on new output; disengages when user scrolls up.
- **FR-005a**: The scrollbar thumb position and size are derived from the current viewport height and scrollback position so the indicator matches the visible slice.
- **FR-005b**: While the user is viewing an older snapshot-backed frame, new output appends to the history cache without forcing the viewport back to live until the user scrolls down to the newest frame.
- **FR-005c**: Snapshot-backed scrollbar metrics use the visible viewport height plus the number of extra historical frames so the thumb length stays proportional to the pane instead of shrinking to a single cell.
- **FR-005d**: PTY output chunks drained in the same event-loop pass are coalesced per session before they enter the app update path so snapshot-backed scrollback tracks rendered frames rather than PTY reader chunk boundaries.
- **FR-005e**: Snapshot-backed history stores every distinct VT-interpreted visible frame; in-place redraws and clear+redraw updates append as historical frames when the resulting frame differs from the latest cached one.
- **FR-005f**: Snapshot append decisions are based on final VT screen state (not raw PTY chunk boundaries or overlap heuristics), with consecutive identical frames deduplicated.
- **FR-005i**: Snapshot progression for full-screen panes does not require viewport-shift overlap scoring; any distinct visible frame remains reviewable through snapshot scrollback.
- **FR-005j**: While alternate screen is active, snapshot-backed scrollback is the primary history source even if main-screen row scrollback exists.
- **FR-005k**: Session viewport operations are routed through one cache-backed visible-surface API, and renderer / URL detection / selection copy all consume that same surface.
- **FR-005l**: Any key input forwarded to the active PTY returns the session viewport to live-follow first, so command input never stays attached to a stale historical viewport.
- **FR-005m**: Agent panes that own terminal scrolling, including SGR-mouse-enabled panes and Codex panes, receive wheel / trackpad scroll input through the PTY instead of gwt-local scrollback so the agent remains the source of truth for redraw and scroll state.
- **FR-005n**: While an agent pane is using PTY-owned scrolling, gwt suppresses its local scrollbar overlay rather than showing a stale thumb derived from unrelated local snapshot history.
- **FR-005g**: Snapshot-backed history prunes leading blank frames whenever newer non-blank frames exist so the oldest reachable viewport is never an empty phantom frame.
- **FR-005h**: Snapshot scroll navigation from live-follow applies exact one-step deltas; the first upward step from live lands on `latest - 1` without off-by-one skipping.
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
- **SC-012**: Snapshot-backed scrollback no longer reveals partially painted intermediate states that existed only between PTY reader chunks within the same drain pass.
- **SC-013**: In-place redraws that clear or overwrite the same visible rows render correctly in the newest cached frame, while older distinct frames remain reviewable via snapshot scrollback.
- **SC-014**: Scrolling to the oldest snapshot no longer yields an empty phantom frame after a blank-to-bottom-aligned first draw transition.
- **SC-015**: Even if a blank frame was previously captured, subsequent non-blank frames cause the blank history prefix to be pruned, so scrolling to the top still renders visible content.
- **SC-016**: First upward scroll from live snapshot mode moves exactly one snapshot backward; frame skipping on live-to-history transition is eliminated.
- **SC-017**: Leaked SGR wheel reports remain normalized even when characters arrive with short gaps; literal `[<...M` artifacts no longer surface in pane output.
- **SC-018**: Even when Terminal pane was not focused before scrolling, leaked SGR wheel sequences are recovered as mouse scroll input and do not leak into pane text.
- **SC-019**: Snapshot history advances for any distinct full-screen frame even under overlap-row churn, preventing practical scrollback starvation on dynamic panes.
- **SC-020**: Alternate-screen panes remain scrollable through snapshot history even when legacy main-screen row scrollback exists; scrollbar and visible frame stay in sync.
- **SC-021**: Viewport movement updates scrollbar, rendered text, URL hit-tests, and copy selection consistently from one visible cache surface, with no source mismatch between features.
- **SC-022**: Agent-pane scrollback preserves VT-derived color and text attributes throughout in-memory history navigation without switching to transcript/session-log fallback.
- **SC-023**: Agent panes whose output redraws full-screen frames without producing vt100 row scrollback still remain scrollable through pane-local in-memory snapshot history; row-only regressions are prevented.
- **SC-024**: While browsing row or snapshot history, any PTY-bound key input returns the viewport to the live screen before the input is forwarded.
- **SC-025**: When an agent pane owns terminal scrolling, mouse-wheel and Terminal.app right-drag fallback input are forwarded to the PTY as SGR wheel events, and gwt does not try to reinterpret that interaction as local scrollback.
- **SC-026**: While PTY-owned scrolling is active, the gwt scrollbar overlay is hidden so the pane no longer shows a misleading thumb that does not track the agent-controlled viewport.
