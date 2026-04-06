# Terminal Emulation -- Tasks

## Phase 1: URL Detection and Opening

- [x] T001 [P] Write RED tests for URL regex matching: single URL, multiple URLs per line, URLs with query params/fragments/parentheses, non-URL text.
- [x] T002 [P] Write RED tests for URL region tracking: verify screen coordinates map to correct URL strings.
- [x] T003 Implement URL regex matching utility that returns match ranges for a given string.
- [x] T004 Integrate URL detection into renderer: scan visible lines, apply underline style to URL regions.
- [x] T005 Implement URL region coordinate tracking for click detection.
- [x] T006 Write RED test for Ctrl+click on URL: verify platform opener is invoked with correct URL.
- [x] T007 Implement Ctrl+click handler: detect click within URL region, invoke `open`/`xdg-open` with URL.
- [x] T008 Write RED test for wrapped URLs spanning two terminal lines.
- [x] T009 Implement wrapped URL detection across adjacent lines.
- [x] T010 Verify all URL detection tests pass GREEN.

## Phase 2: Alt-Screen Buffer Verification

- [x] T011 [P] Write RED test: send DECSET 1049, write alt-screen content, send DECRST 1049, verify main scrollback preserved.
- [x] T012 [P] Write RED test: verify cursor position restores correctly after alt-screen exit.
- [x] T013 Run alt-screen tests against vt100 crate and verify GREEN (or document gaps).
- [x] T014 If gaps found, add workaround or document limitation in spec. *(no gaps found; vt100 handles alt-screen correctly)*

## Phase 3: Regression and Polish

- [x] T015 Run full existing test suite (17+ keybind tests, viewport tests) and verify no regressions.
- [x] T016 Run `cargo clippy` and `cargo fmt` on all changed files.
- [x] T017 Update SPEC-1 progress artifacts with verification results.

## Phase 4: Scrollback Interaction, Selection, And Scrollbar

- [x] T018 [P] Write RED test: mouse-wheel scroll moves the terminal viewport into scrollback and keeps live-follow disabled until the viewport returns to the bottom.
- [x] T019 [P] Write RED test: overflowing terminal history renders a right-edge scrollbar, while non-overflowing history does not reserve scrollbar chrome.
- [x] T020 [P] Write RED test: drag selection across visible scrollback copies the expected single-line and multi-line payload via `contents_between()`.
- [x] T021 Extend terminal session state with viewport and selection tracking in `model.rs`.
- [x] T022 Implement mouse-wheel scrollback control and live-follow restoration in `app.rs`.
- [x] T023 Implement drag-selection extraction/highlighting and clipboard copy in `app.rs` / `renderer.rs`.
- [x] T024 Render the overflow-only terminal scrollbar in the session surface and keep cursor placement correct when the gutter is present.
- [x] T025 Refresh SPEC-1 artifacts and rerun focused terminal interaction verification.
- [x] T026 Fix regression: session-pane mouse wheel now re-focuses the terminal before applying scrollback.
- [x] T027 Fix Terminal.app trackpad regression by disabling alternate-scroll mode during terminal startup and verify with focused startup + scroll/copy regression tests.
