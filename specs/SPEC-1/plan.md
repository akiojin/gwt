# Terminal Emulation -- Implementation Plan

## Summary

Extend the existing terminal emulation layer with URL detection/opening and verify alt-screen buffer correctness. The core rendering pipeline (vt100 parsing, color mapping, scrollback, selection) is already implemented and tested.

## Technical Context

- **Renderer**: `crates/gwt-tui/src/renderer.rs` -- converts vt100 screen to ratatui Buffer
- **vt100 crate**: Handles ANSI parsing, screen buffer, alt-screen (DECSET 1049)
- **Scrollback**: Managed in the pane model, 10,000-line default
- **Selection**: Mouse drag tracking in keybind/input handler, reversed-video in renderer
- **Existing tests**: 17+ keybind tests, viewport scroll tests, color mapping tests

## Constitution Check

- Spec before implementation: yes, this SPEC documents all terminal emulation requirements.
- Test-first: URL detection and alt-screen tests must be RED before implementation.
- No workaround-first: URL detection uses proper regex parsing, not ad-hoc string matching.
- Minimal complexity: URL detection is a rendering overlay; does not modify the core vt100 pipeline.

## Complexity Tracking

- Added complexity: URL regex matching per rendered line, click handler routing
- Mitigation: URL detection runs only on visible lines, not full scrollback

## Phased Implementation

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

## Dependencies

- `vt100` crate: alt-screen support is built-in, no upstream changes needed.
- `regex` crate: already a transitive dependency, can be used for URL pattern matching.
- Platform browser opening: `open` crate or direct command invocation.
