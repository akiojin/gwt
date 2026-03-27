# gwt TUI Migration Design: Tauri GUI to ratatui TUI

## Context

gwt is a SPEC-driven agent management tool. Users launch coding agents (Claude Code, Codex, Gemini) against SPECs, with git worktrees providing isolated workspaces behind the scenes. The current frontend is a Tauri v2 + Svelte 5 desktop GUI introduced in v7.0.0.

This design replaces the GUI with a ratatui-based TUI, making gwt a terminal application that serves as a terminal replacement with its own tab management. The motivation: a TUI is a more natural fit for a tool that manages terminal-based coding agents. Users launch gwt instead of a terminal app, and manage all agents from within it.

## Architecture

```text
+-------------------------------------------+
|              gwt (binary)                 |
+-------------------------------------------+
|          gwt-tui (new crate)              |
|  +----------+-----------+------------+    |
|  | Renderer | KeyBind   | TabManager |    |
|  | (ratatui)| (crossterm)|            |    |
|  +----+-----+-----+-----+------+----+    |
|       |           |            |          |
+-------+-----------+------------+----------+
|             gwt-core (existing)           |
|  +---------+---------+--------+---------+ |
|  |terminal | git     | agent  | config  | |
|  | (PTY)   |(worktree)|       |         | |
|  +---------+---------+--------+---------+ |
+-------------------------------------------+
```

### Crate Responsibilities

- **gwt-tui**: TUI rendering (ratatui), keyboard input (crossterm), tab/pane layout, management panel, event loop. Pure UI layer.
- **gwt-core**: PTY management, git/worktree operations, agent integration, config, AI session summaries. Unchanged.
- **gwt-tauri + gwt-gui**: Deleted after migration.

### Logic Migration from gwt-tauri to gwt-core

Business logic currently in gwt-tauri that must move to gwt-core:

- Agent launch parameter construction -> `gwt-core::agent`
- PR/CI status polling -> `gwt-core::git` (new module)
- Session summary generation trigger -> `gwt-core::ai`
- Voice input runtime management -> `gwt-core` (new module)

## TUI Layout

### Normal Mode

```text
+--[* claude:feature/x]--[* codex:fix/y]--[o shell]--[+]--+
|                                                           |
|  Active tab terminal output (full-screen PTY)             |
|                                                           |
|  $ claude code --model opus ...                           |
|  > Analyzing codebase...                                  |
|                                                           |
+-----------------------------------------------------------+
| gwt | Tab 1/3 | * running | feature/x | SPEC-42          |
+-----------------------------------------------------------+
```

- **Tab bar** (top): agent name + branch, status color (running/idle/error)
- **Terminal area**: active tab PTY output, full screen
- **Status bar** (bottom): current tab info, SPEC association, agent state

### Split Mode

Horizontal and vertical splits supported (tmux-style):

```text
+--[* claude]--[* codex]--[o shell]--[+]-------------------+
| claude:feature/x          | codex:fix/y                   |
|                           |                               |
| > Implementing auth...    | > Running tests...            |
|                           |                               |
+---------------------------+-------------------------------+
| gwt | Split 2 | SPEC-42                                   |
+-----------------------------------------------------------+
```

### Management Panel (Ctrl+G toggle)

```text
+-- MANAGEMENT -------------------------------------------------+
| +-- Agents ---------+-- Detail ----------------------------- +|
| | * claude  [run]   | Agent: Claude Code v1.0.46             ||
| | * codex   [run]   | Branch: feature/x                     ||
| | o shell   [idle]  | Worktree: /tmp/gwt/feature-x          ||
| |                   | SPEC: SPEC-42                          ||
| | [n] New Agent     | Status: running (2h 15m)              ||
| | [s] New Shell     | PR: #1234 (checks passing)            ||
| |                   |                                        ||
| |-- Quick Actions --| [k] Kill  [r] Restart  [l] Logs      ||
| | [p] PR Status     | [Enter] Switch to tab                 ||
| | [i] Issues        |                                        ||
| | [S] SPECs         |-- AI Summary --------------------------||
| |                   | Implementing authentication module...  ||
| |                   | 3 files modified, 2 tests passing      ||
| +-------------------+----------------------------------------+|
+---------------------------------------------------------------+
```

Activated by Ctrl+G. Dismisses on Ctrl+G again or Escape.

## Key Bindings

| Key | Action |
|-----|--------|
| `Ctrl+G` | Toggle management panel |
| `Ctrl+G, n` | New agent tab (launch dialog) |
| `Ctrl+G, s` | New shell tab |
| `Ctrl+G, 1-9` | Switch to tab by number |
| `Ctrl+G, ]` / `[` | Next / previous tab |
| `Ctrl+G, v` | Vertical split |
| `Ctrl+G, h` | Horizontal split |
| `Ctrl+G, x` | Close current tab |
| `Ctrl+G, q` | Quit gwt |

`Ctrl+G` is a prefix key. All other input passes directly to the active PTY.

## Component Design

### gwt-tui Crate Structure

```text
crates/gwt-tui/src/
  main.rs              -- Entry point, tokio runtime
  app.rs               -- App struct (main event loop)
  ui/
    mod.rs
    tab_bar.rs         -- Tab bar rendering
    terminal_view.rs   -- PTY output to ratatui Frame
    status_bar.rs      -- Status bar
    split_layout.rs    -- Split pane management
    management/
      mod.rs           -- Management panel orchestration
      agent_list.rs    -- Agent list
      detail_panel.rs  -- Agent detail
      pr_dashboard.rs  -- PR status
      issue_panel.rs   -- Issue/SPEC list
      launch_dialog.rs -- New agent launch dialog
  input/
    mod.rs
    keybind.rs         -- Ctrl+G prefix key processing
    voice.rs           -- Voice input integration
  state.rs             -- TUI application state
  event.rs             -- Event handler (key, PTY output, timer)
  renderer.rs          -- VT100 buffer -> ratatui Cell conversion
```

### Data Flow

```text
Key input -> crossterm::event -> input/keybind.rs
  +-- Ctrl+G prefix -> management panel / tab operations
  +-- Otherwise -> active tab PTY write_input()

PTY output -> gwt-core PaneManager -> event channel
  -> renderer.rs (VT100 -> ratatui conversion)
  -> ui/terminal_view.rs (draw to Frame)

Timer (100ms) -> UI redraw loop
  -> Agent status polling
  -> PR/Issue status update (low frequency)
```

### State Management

```rust
pub struct TuiState {
    // Tab management
    tabs: Vec<TabInfo>,
    active_tab: usize,
    layout: LayoutTree,  // Split layout tree

    // Management panel
    management_visible: bool,
    management_focus: ManagementSection,

    // Ctrl+G prefix
    prefix_active: bool,

    // gwt-core references
    pane_manager: PaneManager,
    // agent, git, config used directly from gwt-core
}
```

### VT100 to ratatui Rendering

gwt-core's `terminal/emulator.rs` maintains VT100 state. `renderer.rs` converts the cell buffer to ratatui `Buffer` for drawing. This pattern was proven in the v6.x TUI.

## Worktree Handling

Worktrees are backend infrastructure, not user-facing. The user interacts with agents and SPECs. Worktrees are:

- **Auto-created** when an agent is launched with an Issue/branch specification
- **Auto-cleaned** when an agent tab is closed (with safety checks)
- **Invisible** in normal operation; visible only in the management panel's agent detail as context

## Features

### Core (Phase 1-2)

- Tab management (create, close, switch, reorder)
- Full PTY terminal rendering with color/attribute support
- Ctrl+G management panel with agent list and detail
- Agent launch (Claude Code, Codex, Gemini) with Worktree auto-creation
- Shell tab support
- Split panes (horizontal/vertical)

### Extended (Phase 3-4)

- PR dashboard (status, CI checks, merge state)
- Issue/SPEC management panel
- AI session summaries (scrollback analysis)
- Voice input integration (Qwen3-ASR)

## Scrollback Buffer

Each PTY tab has a scrollback buffer, backed by gwt-core's `terminal/scrollback.rs` (file-based persistence at `~/.gwt/terminals/{id}.log`). In the TUI:

- Normal mode: terminal shows live PTY output at the bottom of the scrollback
- Scroll mode (activated by `Ctrl+G, PgUp` or mouse wheel): freezes the viewport and allows scrolling through history
- Exiting scroll mode (press `q` or `Escape`): returns to live output
- Scrollback is preserved across tab switches
- AI summary generation reads from the scrollback buffer

## Error Handling

- **PTY crash**: Status bar notification, tab persists with restart option
- **Worktree creation failure**: Error shown in management panel, manual retry available
- **GitHub API error**: PR/Issue panel shows offline state, background retry
- **Terminal too small**: Warning when below 80x24 minimum

## Testing Strategy

- **gwt-core**: Existing tests unchanged
- **gwt-tui**:
  - `renderer.rs`: Unit tests for VT100-to-ratatui cell conversion (color, attributes)
  - `keybind.rs`: Key sequence parse and dispatch tests
  - `state.rs`: State transition tests (tab ops, splits, management toggle)
  - Integration: ratatui `TestBackend` snapshot tests

## Migration Phases

| Phase | Scope | Deliverable |
|-------|-------|-------------|
| 0 | Setup | gwt-tui crate in workspace, gwt-core dependency |
| 1 | Minimal TUI | Tab switching + PTY display + status bar |
| 2 | Management | Ctrl+G panel, agent launch/stop, shell tabs |
| 3 | Splits | Horizontal/vertical pane splitting |
| 4 | Extended | PR dashboard, Issue/SPEC panel, AI summaries |
| 5 | Voice | Voice input integration |
| 6 | Cleanup | Delete gwt-tauri + gwt-gui, update CI/release |

Each phase produces a working artifact for incremental validation.

## Technology

- **TUI framework**: ratatui (latest) + crossterm (latest)
- **Async runtime**: tokio (existing in gwt-core)
- **PTY**: portable-pty v0.9 (existing in gwt-core)
- **VT100 emulation**: vt100 crate (existing in gwt-core)
- **Terminal state**: gwt-core PaneManager (existing, 5,700+ lines)
