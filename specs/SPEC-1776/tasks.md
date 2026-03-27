# Tasks: SPEC-1776 — Migrate from Tauri GUI to ratatui TUI

## Phase 0: Setup

- [x] T001: Add `crates/gwt-tui/` to Cargo workspace members in `Cargo.toml`
- [x] T002: Create `crates/gwt-tui/Cargo.toml` with dependencies: ratatui, crossterm, tokio, gwt-core
- [x] T003: Create `crates/gwt-tui/src/main.rs` — crossterm raw mode init, ratatui Terminal, empty event loop, graceful shutdown
- [x] T004: Verify `cargo build -p gwt-tui` and `cargo clippy -p gwt-tui` pass

## Phase 1: Foundational — Minimal TUI (US1, US3)

### Renderer (FR-016)

- [x] T010: Write tests for VT100 Cell → ratatui Cell color mapping in `crates/gwt-tui/src/renderer.rs` (named, indexed, RGB colors + bold/italic/underline/inverse attributes)
- [x] T011: Implement `renderer.rs` — convert vt100::Screen grid to ratatui Buffer
- [x] T012: [P] Write snapshot tests for renderer with multi-color PTY output samples

### State & Event Loop

- [x] T020: Write tests for `crates/gwt-tui/src/state.rs` — TuiState tab add/remove/switch, active index bounds
- [x] T021: Implement `state.rs` — TuiState struct with tabs, active_tab, layout, prefix state
- [x] T022: Write tests for `crates/gwt-tui/src/event.rs` — event polling, PTY output channel dispatch
- [x] T023: Implement `event.rs` — crossterm event reader + PTY output channel + tick timer (100ms)

### Key Binding (FR-004)

- [x] T030: Write tests for `crates/gwt-tui/src/input/keybind.rs` — Ctrl+G prefix detection, timeout, action dispatch, passthrough
- [x] T031: Implement `input/keybind.rs` — prefix state machine (Idle → PrefixActive → action/timeout/cancel)

### UI Components (FR-002, FR-003, FR-009, FR-010)

- [x] T040: [P] Implement `crates/gwt-tui/src/ui/tab_bar.rs` — tab names, status colors (AgentColor mapping), active indicator
- [x] T041: [P] Implement `crates/gwt-tui/src/ui/terminal_view.rs` — render VT100 buffer via renderer to Frame area
- [x] T042: [P] Implement `crates/gwt-tui/src/ui/status_bar.rs` — tab index, agent state, branch, SPEC ID
- [x] T043: Write snapshot tests for tab_bar, terminal_view, status_bar using ratatui TestBackend

### App Integration (FR-007)

- [x] T050: Implement `crates/gwt-tui/src/app.rs` — App struct orchestrating state + event + UI render cycle
- [x] T051: Wire shell tab creation (Ctrl+G,c) via PaneManager::spawn_shell()
- [x] T052: Wire PTY I/O: key input → write_input(), PTY reader → process_bytes() → render
- [x] T053: Wire terminal resize event → PaneManager::resize_all() + re-render
- [ ] T054: Implement scrollback scroll mode (Ctrl+G,PgUp to enter, Escape to exit)

### Phase 1 Verification

- [ ] T060: Integration test — launch gwt-tui, open shell tab, verify PTY output renders with ANSI colors
- [x] T061: Verify `cargo test -p gwt-tui` and `cargo test -p gwt-core` both pass

## Phase 2: Agent Tabs + Management Panel (US2, US4)

### Business Logic Extraction

- [ ] T100: Extract agent launch parameter builder from `crates/gwt-tauri/src/commands/terminal.rs` to `crates/gwt-core/src/agent/launch.rs`
- [ ] T101: Write tests for extracted launch builder in gwt-core
- [ ] T102: [P] Extract session completion watcher from `crates/gwt-tauri/src/session_watcher.rs` to `crates/gwt-core/src/agent/session_watcher.rs`

### Launch Dialog (FR-006)

- [ ] T110: Write tests for `crates/gwt-tui/src/ui/management/launch_dialog.rs` — field navigation, agent type selection, input validation
- [ ] T111: Implement `launch_dialog.rs` — agent type selector, branch/Issue input, directory picker, confirm/cancel

### Management Panel (FR-005)

- [ ] T120: Write tests for `crates/gwt-tui/src/ui/management/agent_list.rs` — list rendering, cursor navigation, status indicators
- [ ] T121: Implement `agent_list.rs` — agent list with status color (running/idle/error), selection cursor
- [ ] T122: [P] Write tests for `crates/gwt-tui/src/ui/management/detail_panel.rs` — detail fields display
- [ ] T123: [P] Implement `detail_panel.rs` — agent name, branch, worktree path, SPEC, status, uptime, PR link
- [ ] T124: Implement `crates/gwt-tui/src/ui/management/mod.rs` — panel layout (left list + right detail), Ctrl+G toggle

### Agent Tab Lifecycle (FR-015)

- [ ] T130: Wire Ctrl+G,n → launch_dialog → PaneManager::launch_agent() with auto-worktree
- [ ] T131: Wire management panel quick actions: k (kill), r (restart), Enter (switch to tab)
- [ ] T132: Implement worktree auto-cleanup on agent tab close (with uncommitted changes safety check)

### Phase 2 Verification

- [ ] T140: Integration test — launch agent, verify appears in management panel, kill via panel, worktree cleaned up
- [ ] T141: Verify all tests pass: `cargo test -p gwt-tui && cargo test -p gwt-core`

## Phase 3: Split Panes (US5)

### Split Layout (FR-008)

- [ ] T200: Write tests for `crates/gwt-tui/src/ui/split_layout.rs` — LayoutTree insert split, remove leaf, resize ratio, area calculation
- [ ] T201: Implement `split_layout.rs` — binary tree layout with H/V splits, ratio-based area subdivision
- [ ] T202: Wire Ctrl+G,v (vertical split) and Ctrl+G,h (horizontal split)
- [ ] T203: Implement pane focus switching within split layout (Ctrl+G,arrow keys)
- [ ] T204: Wire terminal resize → recalculate all split pane areas

### Phase 3 Verification

- [ ] T210: Snapshot test — two panes in vertical split render correct areas
- [ ] T211: Integration test — split, resize terminal, verify both panes update

## Phase 4: Extended Features (US6, US7)

### PR Dashboard (FR-011)

- [ ] T300: [P] Extract PR status polling from `crates/gwt-tauri/src/commands/` to `crates/gwt-core/src/git/pr_status.rs`
- [ ] T301: [P] Write tests for PR status module in gwt-core
- [ ] T302: Implement `crates/gwt-tui/src/ui/management/pr_dashboard.rs` — PR list, CI check badges, merge state

### Issue/SPEC Panel (FR-012)

- [ ] T310: [P] Implement `crates/gwt-tui/src/ui/management/issue_panel.rs` — Issue/SPEC list with search input
- [ ] T311: Wire Issue search to gwt-core's ChromaDB index (existing gwt-issue-search infrastructure)

### AI Session Summaries (FR-013)

- [ ] T320: [P] Extract summary trigger from gwt-tauri to `crates/gwt-core/src/ai/summary_trigger.rs`
- [ ] T321: Display AI summary in management detail panel, updated periodically from scrollback

### Phase 4 Verification

- [ ] T330: Verify PR status displays in management panel
- [ ] T331: Verify Issue search returns results
- [ ] T332: Verify AI summary generates from agent scrollback

## Phase 5: Voice Input (US8)

### Voice Runtime (FR-014)

- [ ] T400: Extract voice runtime from `crates/gwt-tauri/src/commands/voice.rs` to `crates/gwt-core/src/voice/runtime.rs`
- [ ] T401: Write tests for voice runtime initialization and transcription pipeline
- [ ] T402: Implement `crates/gwt-tui/src/input/voice.rs` — hotkey activation, audio capture, inject transcribed text to active PTY

### Phase 5 Verification

- [ ] T410: Manual test — voice input hotkey → speak → text appears in terminal

## Phase 6: Cleanup + Release (SC-007, SC-008)

### Code Removal

- [ ] T500: Delete `crates/gwt-tauri/` directory
- [ ] T501: Delete `gwt-gui/` directory
- [ ] T502: Update `Cargo.toml` workspace members (remove gwt-tauri, keep gwt-tui)
- [ ] T503: Remove Tauri-specific dependencies from workspace Cargo.toml
- [ ] T504: Update binary target in Cargo.toml to point to gwt-tui

### CI/CD Pipeline Updates

- [ ] T510: Update `.github/workflows/test.yml` — remove vitest job, remove Playwright E2E job, remove Tauri WebDriver E2E job, keep `cargo test -p gwt-core -p gwt-tui`
- [ ] T511: [P] Update `.github/workflows/release.yml` — replace `cargo tauri build` with `cargo build --release -p gwt-tui`, remove pnpm/Node steps, remove macOS signing/notarization, remove Windows MSI builder, add cross-compile (Linux x86_64/aarch64, macOS universal, Windows x86_64)
- [ ] T512: [P] Update `.github/workflows/lint.yml` — remove `svelte-check` job, keep Clippy/Rustfmt/markdownlint/commitlint
- [ ] T513: [P] Update `.github/workflows/coverage.yml` — remove frontend coverage job, remove `frontend` Codecov flag, keep Rust `cargo llvm-cov`
- [ ] T514: Update `.github/workflows/voice-eval.yml` — update paths if gwt-tui changes affect voice module location
- [ ] T515: Remove `tauri.conf.json` and related Tauri config files
- [ ] T516: Delete `installers/` directory (macOS .dmg builder, Windows .msi builder)

### Playwright E2E Removal + TUI Test Replacement

- [ ] T517: Delete `gwt-gui/e2e/` (22 Playwright test files)
- [ ] T518: Delete `gwt-gui/e2e-tauri/` (Tauri WebDriver tests)
- [ ] T519: Delete `gwt-gui/playwright.config.ts`
- [ ] T520: Add ratatui TestBackend snapshot tests for all TUI screens (welcome, single pane, split, management panel)
- [ ] T521: Add PTY integration tests — spawn gwt-tui subprocess, send keystrokes, verify output

### Documentation

- [ ] T530: [P] Update `README.md` — installation = download binary, remove Tauri/GUI references
- [ ] T531: [P] Update `README.ja.md` — same changes in Japanese
- [ ] T532: Update `CLAUDE.md` — remove Tauri/GUI references, add TUI development instructions, update build/test commands

### Phase 6 Verification

- [ ] T540: `cargo build -p gwt-tui` succeeds as the sole frontend
- [ ] T541: `cargo test` (all crates) passes
- [ ] T542: `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] T543: CI pipeline runs successfully on all platforms
- [ ] T544: Release workflow produces correct cross-platform binary artifacts
- [ ] T545: TUI snapshot tests cover all defined screens
