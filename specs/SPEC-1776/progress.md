# Progress: SPEC-1776

## 2026-04-01: Constitution path unified under `.gwt`

### Progress

- Found a root-cause mismatch: runtime logic already treated `.gwt/memory/constitution.md` as canonical, but `gwt-core` still embedded `memory/constitution.md` at compile time
- Switched the managed asset source to `.gwt/memory/constitution.md` and stopped counting the legacy root path as satisfying registration status
- Removed the tracked duplicate `memory/constitution.md` file and updated stale path references in source comments and SPEC docs

### Done

- Skill registration now has a single canonical constitution source: `.gwt/memory/constitution.md`

### Next

- Verify on a clean checkout that `cargo test -p gwt-core -p gwt-tui` passes without recreating `memory/constitution.md`

## 2026-04-01: PTY paste input

### Progress

- Confirmed that `Enter` and pasted text were on different paths: `Enter` was normalized, but terminal `Paste` events were ignored
- Enabled bracketed paste at the terminal boundary and routed `Event::Paste(String)` through the Elm update loop
- Added a PTY integration test proving that multi-line pasted text is forwarded to the active pane as one payload

### Done

- Text paste into Agent/Shell tabs now reaches the PTY reliably, including embedded newlines

### Next

- Manual E2E: paste multi-line text into an Agent/Shell tab in Terminal.app and confirm the full payload arrives without splitting into per-key behavior

## 2026-04-01: Main PTY copy mode

### Progress

- Added a dedicated PTY copy mode on `Ctrl+G,m` for the active Agent/Shell tab
- Kept terminal-native selection/copy in normal mode by enabling mouse capture only in management screens or copy mode
- Added keyboard scrollback navigation, mouse wheel scrolling, drag-to-copy, and viewport freeze while PTY output continues
- Removed stale `Ctrl+G,n` launch shortcut references so agent launch stays anchored on Branches `Enter`

### Done

- Main PTY now supports explicit copy/scroll behavior without regressing terminal-native copy outside copy mode

### Next

- Manual E2E: enter copy mode in an Agent/Shell tab, scroll with trackpad, drag-copy text, then exit and verify the viewport snaps back to live output

## 2026-04-01: Logs trackpad scroll fix

### Progress

- Routed `MouseInput` scroll events to the Logs screen navigation handler
- Added RED/GREEN coverage for `ScrollUp` and `ScrollDown` on the Logs tab
- Cleaned package-local clippy failures encountered during verification

### Done

- Trackpad / mouse wheel scrolling now moves the Logs selection so older entries are reachable

### Next

- Manual E2E: open Logs tab and confirm trackpad scrolling moves through historical entries
## 2026-04-01: Session auto-close on exit

### Progress

- Added PTY termination polling in `Model::apply_background_updates()`
- Automatically close Agent and Shell tabs when their underlying process exits
- Removed the previous Completed-tab retention behavior from the session lifecycle

### Done

- Exited Agent and Shell sessions now close their tabs automatically

### Next

- Manual E2E: launch agent/shell, exit the process, confirm tab disappears cleanly

## 2026-03-27: Phase 0 + Phase 1 Core Implementation

### Progress

- Created `crates/gwt-tui/` crate with ratatui 0.29, crossterm 0.28, vt100 0.15
- Implemented all Phase 0 tasks (T001-T004): scaffold, Cargo.toml, main.rs, build verification
- Implemented Phase 1 renderer (T010-T012): VT100 Screen → ratatui Buffer with color/attribute mapping
- Implemented Phase 1 state (T020-T021): TuiState with tab management, bounds-safe navigation
- Implemented Phase 1 event (T022-T023): EventLoop multiplexing crossterm + PTY channel + tick timer
- Implemented Phase 1 keybind (T030-T031): Ctrl+G prefix state machine with 2s timeout, full action set
- Implemented Phase 1 UI components (T040-T043): tab_bar, terminal_view, status_bar with snapshot tests
- Implemented Phase 1 app (T050): App struct with event dispatch and render cycle
- 53 tests passing, clippy clean, gwt-core tests unaffected

### Done

- Phase 0: Complete (T001-T004)
- Phase 1 Renderer: Complete (T010-T012)
- Phase 1 State + Event: Complete (T020-T023)
- Phase 1 KeyBind: Complete (T030-T031)
- Phase 1 UI Components: Complete (T040-T043)
- Phase 1 App skeleton: Complete (T050)
- Phase 1 Verification partial: T061 complete (all tests pass)

### Next

- T051: Wire shell tab creation (Ctrl+G,c) via PaneManager::spawn_shell()
- T052: Wire PTY I/O (key → write_input, PTY reader → process_bytes → render)
- T053: Wire terminal resize → PaneManager::resize_all()
- T054: Implement scrollback scroll mode
- T060: Integration test with live PTY
