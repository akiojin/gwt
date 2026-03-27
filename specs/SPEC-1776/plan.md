# Implementation Plan: SPEC-1776 — Migrate from Tauri GUI to ratatui TUI

## Summary

Replace gwt's Tauri v2 + Svelte 5 GUI with a ratatui + crossterm TUI. Create a new `gwt-tui` crate that serves as the frontend, using gwt-core's existing terminal/git/agent/config APIs unchanged. Delete `gwt-tauri` and `gwt-gui` after migration.

## Technical Context

### Existing Infrastructure (gwt-core)

- **PaneManager**: Multi-pane lifecycle management with `launch_agent()`, `spawn_shell()`, tab navigation, resize
- **TerminalPane**: PTY I/O via `write_input()`/`process_bytes()`, status tracking, scrollback persistence
- **PtyHandle**: Cross-platform PTY via portable-pty v0.9 (macOS, Linux, Windows ConPTY)
- **AgentColor**: Color enum with named colors + RGB + indexed (maps directly to ratatui::Color)
- **Git/Worktree**: Full worktree management, branch operations, Issue linking
- **Agent**: Claude/Codex/Gemini integration, session store, scanner
- **Config**: Settings, profiles, skills, recent projects

### New Dependencies

- `ratatui` (latest): TUI rendering framework
- `crossterm` (latest): Terminal I/O backend
- `vt100` (existing in gwt-core): VT100 emulator for PTY output parsing

### Key Decision

gwt-tui is a thin UI layer. All business logic lives in gwt-core. Logic currently in gwt-tauri's command handlers that belongs in core (agent launch flow, PR polling, session summary triggers) will be extracted to gwt-core modules.

## Constitution Check

No `memory/constitution.md` found. Checked against CLAUDE.md rules:

- **Simplicity**: TUI is simpler than Tauri GUI (no IPC, no web stack, no Svelte). Compliant.
- **TDD**: Each phase starts with tests. Compliant.
- **SPEC required**: This SPEC satisfies the requirement. Compliant.
- **No workarounds**: Clean replacement, not a patch. Compliant.
- **Existing file maintenance**: gwt-core files are maintained, not duplicated. Compliant.

## Project Structure

```text
crates/
  gwt-core/         # Unchanged (except business logic extracted from gwt-tauri)
  gwt-tui/          # NEW — replaces gwt-tauri
    Cargo.toml
    src/
      main.rs
      app.rs
      state.rs
      event.rs
      renderer.rs
      ui/
        mod.rs
        tab_bar.rs
        terminal_view.rs
        status_bar.rs
        split_layout.rs
        management/
          mod.rs
          agent_list.rs
          detail_panel.rs
          pr_dashboard.rs
          issue_panel.rs
          launch_dialog.rs
      input/
        mod.rs
        keybind.rs
        voice.rs
  gwt-tauri/         # DELETED in Phase 6
gwt-gui/             # DELETED in Phase 6
```

## Complexity Tracking

| Risk | Mitigation |
|------|-----------|
| VT100→ratatui rendering fidelity | Reuse v6.x proven pattern; extensive snapshot tests |
| Ctrl+G prefix key vs PTY passthrough | Strict state machine; never forward Ctrl+G |
| Cross-platform PTY differences | Already handled by gwt-core portable-pty |
| Business logic extraction from gwt-tauri | Incremental; each moved piece gets its own test |
| Split pane resize math | Dedicated module with property-based tests |

## Phased Implementation

### Phase 0: Scaffold (FR-001)

**Goal**: gwt-tui crate exists in workspace, compiles, shows empty TUI.

- Add `crates/gwt-tui/` to Cargo workspace
- Cargo.toml with dependencies: ratatui, crossterm, tokio, gwt-core
- `main.rs`: Initialize crossterm raw mode, create ratatui Terminal, run event loop
- `app.rs`: App struct with empty render cycle
- Verify: `cargo build -p gwt-tui` succeeds, `cargo run -p gwt-tui` shows blank TUI

### Phase 1: Minimal TUI (FR-002, FR-003, FR-007, FR-009, FR-010, FR-016)

**Goal**: Single shell tab works with full PTY rendering.

- `renderer.rs`: VT100 Cell → ratatui Cell conversion (color mapping, attributes)
- `ui/terminal_view.rs`: Render PTY output buffer to ratatui Frame
- `ui/tab_bar.rs`: Tab bar with name, branch, status color
- `ui/status_bar.rs`: Current tab info
- `state.rs`: TuiState with tabs vector, active index
- `event.rs`: Key input → PTY write, PTY output → process_bytes, resize events
- `input/keybind.rs`: Ctrl+G prefix key detection (passthrough vs intercept)
- Shell tab: Ctrl+G,s spawns shell via PaneManager::spawn_shell()
- Scrollback: Scroll mode via Ctrl+G,PgUp
- Verify: Launch gwt-tui, open shell tab, type commands, see output with colors

### Phase 2: Agent Tabs + Management Panel (FR-004, FR-005, FR-006, FR-015)

**Goal**: Launch agents, toggle management panel.

- `ui/management/launch_dialog.rs`: Agent type selector, branch/Issue input, directory picker
- `ui/management/agent_list.rs`: Agent list with status indicators
- `ui/management/detail_panel.rs`: Selected agent detail (branch, worktree, status, uptime)
- `ui/management/mod.rs`: Panel layout orchestration
- Ctrl+G toggle management panel visibility
- Agent launch: Ctrl+G,n opens dialog → PaneManager::launch_agent() with auto-worktree
- Quick actions: kill (k), restart (r), switch to tab (Enter)
- Extract agent launch parameter builder from gwt-tauri to gwt-core
- Verify: Launch agent, see in management panel, kill/restart, auto-worktree works

### Phase 3: Split Panes (FR-008)

**Goal**: Side-by-side terminal views.

- `ui/split_layout.rs`: LayoutTree (binary tree of splits)
- Ctrl+G,v for vertical split, Ctrl+G,h for horizontal split
- Pane focus switching within splits
- Resize proportional distribution
- Verify: Split two agents side by side, both render correctly, resize works

### Phase 4: Extended Features (FR-011, FR-012, FR-013)

**Goal**: PR dashboard, Issue/SPEC panel, AI summaries.

- Extract PR status polling from gwt-tauri to gwt-core::git (new module)
- Extract session summary trigger from gwt-tauri to gwt-core::ai
- `ui/management/pr_dashboard.rs`: PR status, CI checks, merge state
- `ui/management/issue_panel.rs`: Issue/SPEC search and list
- AI summary: Display in detail panel, periodically updated from scrollback
- Verify: PR status shows in panel, Issues searchable, summaries generate

### Phase 5: Voice Input (FR-014)

**Goal**: Voice input works in TUI.

- Extract voice runtime from gwt-tauri to gwt-core (new module)
- `input/voice.rs`: Hotkey activation, audio capture, Qwen3-ASR transcription
- Transcribed text injected into active PTY
- Verify: Hold hotkey, speak, text appears in terminal

### Phase 6: Cleanup + Release (SC-007, SC-008)

**Goal**: Remove old code, update CI.

- Delete `crates/gwt-tauri/`
- Delete `gwt-gui/`
- Update `Cargo.toml` workspace members
- Update CI workflows (remove Tauri build, add TUI binary build)
- Update release workflow (no .dmg/.msi, just binary)
- Update README.md and README.ja.md
- Verify: Full CI passes, release produces correct binaries
