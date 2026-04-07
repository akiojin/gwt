# Terminal Emulation -- Implementation Plan

## Summary

Complete the terminal emulation layer by first adding a real vt100-backed session surface in `gwt-tui`, then layering URL opening, scrollback interaction, selection, and alt-screen verification on top of it. Low-level renderer tests exist, but the interactive session pane still needs viewport routing, selection, scrollbar handling, and a fallback for full-screen panes whose redraw model leaves vt100 row scrollback at zero.

## Technical Context

- **Renderer**: `crates/gwt-tui/src/renderer.rs` -- converts vt100 screen to ratatui Buffer
- **vt100 crate**: Handles ANSI parsing, screen buffer, alt-screen (DECSET 1049)
- **Current blocker**: agent / full-screen panes can redraw in place without accumulating vt100 row scrollback, so the session pane needs a separate ephemeral snapshot history for recent review
- **Existing tests**: renderer URL tests, alt-screen tests, and broader `gwt-tui` keybind coverage already exist

## Constitution Check

- Spec before implementation: yes, this SPEC documents all terminal emulation requirements.
- Test-first: URL detection and alt-screen tests must be RED before implementation.
- No workaround-first: URL detection uses proper regex parsing, not ad-hoc string matching, and full-screen scrollback uses pane-local snapshots rather than transcript scraping.
- Minimal complexity: URL detection is a rendering overlay; does not modify the core vt100 pipeline.

## Complexity Tracking

- Added complexity: URL regex matching per rendered line, click handler routing, ephemeral screen snapshot buffering
- Mitigation: URL detection runs only on visible lines, not full scrollback, and snapshot history is bounded to a fixed ring buffer

## Phased Implementation

### Phase 0: Session Surface Foundation

1. Store real vt100-backed session state in the model instead of only terminal dimensions.
2. Feed `PtyOutput` into the per-session parser/screen state.
3. Render the session pane through `renderer::render_vt_screen` and preserve URL regions / geometry for hit testing.

### Phase 1: URL Detection and Opening

1. Add URL regex pattern matching utility (`url_detector.rs` or inline in renderer).
2. During rendering, scan visible lines for URL matches and apply underline style.
3. Track URL regions with their screen coordinates for click detection.
4. On Ctrl+click within a URL region, invoke platform browser opener.
5. Add tests: URL pattern matching, multi-URL lines, wrapped URLs, special characters.

### Phase 2: Alt-Screen Buffer Verification

1. Create test fixtures that send DECSET 1049 (enter alt-screen) and DECRST 1049 (exit).
2. Verify main scrollback is preserved after alt-screen round-trip.
3. Verify cursor position and screen content after alt-screen exit.
4. Document any vt100 crate limitations in edge cases.

### Phase 3: Viewport Interaction, Selection, And Scrollbar

1. Extend session terminal state with viewport/follow-live state and the minimum selection state needed to map mouse drag coordinates back into the visible scrollback.
2. Route mouse wheel input into `vt100::Parser::set_scrollback()` and keep new PTY output from snapping the viewport back to live while the user is reviewing history.
3. Use `vt100::Screen::contents_between()` for copy extraction so wrapped rows and wide characters are copied from the rendered viewport contract instead of from ad-hoc transcript slicing.
4. Reserve a right-side gutter only when history overflows the visible pane and render a scrollbar whose thumb matches the current viewport position.
5. During outer-terminal initialization, explicitly disable alternate-scroll mode so Terminal.app does not translate trackpad gestures into cursor-key input while gwt owns the alternate screen.
6. Add a Terminal.app-specific fallback that maps `Down/Drag/Up(Right)` gesture sequences into vertical scrollback deltas because crossterm may not emit `ScrollUp/ScrollDown` for trackpad motion there.
7. For panes whose visible screen has `max_scrollback == 0`, capture distinct live screen states into a pane-local in-memory ring buffer and route wheel scrolling through snapshot history instead of vt100 row scrollback.
8. Keep pane history ephemeral in memory and treat PTY-derived VT state as the only runtime scrollback source for Claude/Codex agent panes.
9. For Claude/Codex agent panes, replace snapshot-frame-based recent scrollback with a normalized row-scrollback parser that strips alternate-screen toggles so launch/blank/status redraws do not become separate history entries.
10. Increase the agent-pane row scrollback capacity above the standard terminal default while keeping it bounded in memory and discarded when the pane closes.
11. Do not hydrate agent-pane runtime scrollback from session `jsonl` or session-log files; agent-side PTY re-output is the only restoration mechanism.

## Dependencies

- `vt100` crate: alt-screen support is built-in, no upstream changes needed.
- `regex` crate: already a transitive dependency, can be used for URL pattern matching.
- Platform browser opening: `open` crate or direct command invocation.
