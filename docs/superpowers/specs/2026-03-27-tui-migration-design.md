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

## Conceptual Model (tmux-style)

```text
gwt
 +-- Window (= tab bar item)
      +-- Pane A (PTY)
      +-- Pane B (PTY)   <- added by split
      +-- Pane C (PTY)   <- further split
```

- **Window** = one tab bar item. Contains one or more panes.
- **Pane** = one PTY session (agent or shell).
- Windows can be split into multiple panes. Without splits, 1 Window = 1 Pane.
- Each Window remembers its last focused pane when switching away.

## Screen Definitions

### Welcome Screen (no windows)

```text
+--[+]----------------------------------------------------------+
|                                                                |
|                       Welcome to gwt                           |
|                                                                |
|       No agents running. Get started:                          |
|                                                                |
|       [n]  Launch new agent                                    |
|       [s]  Open shell                                          |
|       [q]  Quit                                                |
|                                                                |
+----------------------------------------------------------------+
| gwt v9.0.0 | No active windows                                 |
+----------------------------------------------------------------+
```

### Single Pane (default)

```text
+--[W1: * claude]--[W2: * codex]--[W3: o shell]--[+]-----------+
|                                                                |
|  Active Window's active Pane (full-screen PTY)                 |
|                                                                |
|  $ claude code --model opus ...                                |
|  > Analyzing codebase...                                       |
|                                                                |
+----------------------------------------------------------------+
| W1 | Pane 1/1 | * running | feature/x | SPEC-42               |
+----------------------------------------------------------------+
```

### Vertical Split (left/right, same Window)

```text
+--[W1: * split(2)]--[W2: * codex]--[W3: o shell]--[+]---------+
| * claude           | o shell                                   |
| feature/x          | ~/project                                 |
|                    |                                            |
| > Implementing...  | $ cargo test                              |
|                    | running 42 tests...                        |
|                    |                                            |
+----------------------------------------------------------------+
| W1 | Pane 1/2 [focus:L] | * running | feature/x               |
+----------------------------------------------------------------+
```

### Horizontal Split (top/bottom, same Window)

```text
+--[W1: * split(2)]--[W2: o shell]--[+]-------------------------+
| * claude | feature/x                                           |
| > Implementing authentication module...                        |
| > Modified: src/auth.rs, src/middleware.rs                      |
+----------------------------------------------------------------+
| * codex | fix/bug-123                                          |
| > Running tests... 42/42 passed                                |
|                                                                |
+----------------------------------------------------------------+
| W1 | Pane 1/2 [focus:T] | * running | feature/x               |
+----------------------------------------------------------------+
```

### Management Panel (Ctrl+G, Ctrl+G toggle)

```text
+-- MANAGEMENT --------------------------------------------------+
| +-- Windows --------+-- Detail ------------------------------+|
| | W1: * claude      | Window: W1                             ||
| |   +-- 2 panes     | Panes: claude(run), shell(idle)        ||
| | W2: * codex       | Branch: feature/x                     ||
| |   +-- 1 pane      | Worktree: ~/.gwt/wt/feature-x         ||
| | W3: o shell       | SPEC: SPEC-42                          ||
| |   +-- 1 pane      | Uptime: 2h 15m                        ||
| |                    | PR: #1234 (checks passing)             ||
| | [n] New Agent      |                                       ||
| | [s] New Shell      | [k] Kill  [r] Restart                 ||
| | [p] PR Status      | [Enter] Switch to window              ||
| | [i] Issues         |                                       ||
| |                    |-- AI Summary -------------------------||
| |                    | Implementing auth module. Modified     ||
| |                    | 3 files, 2 tests passing.             ||
| +--------------------+---------------------------------------+|
+----------------------------------------------------------------+
```

### Window Navigation Example

```text
State: W1 left pane focused
+--[W1: * split(2)]--[W2: o shell]--+
| [FOCUS]        | pane B            |
| pane A         |                   |
+----------------+-------------------+

Ctrl+G, Right -> W1 right pane focused
+--[W1: * split(2)]--[W2: o shell]--+
| pane A         | [FOCUS]           |
|                | pane B            |
+----------------+-------------------+

Ctrl+G, ] -> Switch to W2 (whole Window changes)
+--[W1: * split(2)]--[W2: o shell]--+
|                                    |
| [FOCUS] shell                      |
| $ _                                |
+------------------------------------+

Ctrl+G, [ -> Back to W1 (remembers last focused pane)
+--[W1: * split(2)]--[W2: o shell]--+
| pane A         | [FOCUS]           |
|                | pane B            |
+----------------+-------------------+
```

## Key Bindings

### Normal Mode (PTY passthrough)

All keystrokes are forwarded to the active pane's PTY.
Only `Ctrl+G` is intercepted as the prefix key.

### Ctrl+G Prefix Mode

Pressing Ctrl+G enters prefix mode (2 second timeout).

**Window (tab) operations:**

| Key | Action |
|-----|--------|
| `Ctrl+G, c` | New Window with empty shell pane |
| `Ctrl+G, n` | New agent Window (launch dialog) |
| `Ctrl+G, 1-9` | Switch to Window by number |
| `Ctrl+G, ]` | Next Window |
| `Ctrl+G, [` | Previous Window |
| `Ctrl+G, &` | Close Window (confirm: kills all panes) |

**Pane operations (within current Window):**

| Key | Action |
|-----|--------|
| `Ctrl+G, v` | Vertical split (new shell pane on right) |
| `Ctrl+G, h` | Horizontal split (new shell pane below) |
| `Ctrl+G, Left/Right/Up/Down` | Move focus to adjacent pane |
| `Ctrl+G, x` | Close active pane (confirm if running) |
| `Ctrl+G, z` | Zoom: toggle active pane fullscreen |

**Scrollback:**

| Key | Action |
|-----|--------|
| `Ctrl+G, PgUp` | Enter scroll mode |
| Mouse wheel / trackpad | Scroll PTY output (enters scroll mode automatically) |

**Management:**

| Key | Action |
|-----|--------|
| `Ctrl+G, Ctrl+G` | Toggle management panel |
| `Ctrl+G, q` | Quit gwt (confirm if agents running) |

**Cancel:**

| Key | Action |
|-----|--------|
| `Escape` | Cancel prefix, return to normal mode |
| 2s timeout | Auto-cancel |

### Scroll Mode

Entered via `Ctrl+G, PgUp` or mouse wheel/trackpad scroll.

| Key | Action |
|-----|--------|
| `PgUp` / `PgDn` | Page scroll |
| `Up` / `Down` | Line scroll |
| Mouse wheel / trackpad | Scroll |
| `q` / `Escape` | Exit scroll mode, return to live output |

### Management Panel

| Key | Action |
|-----|--------|
| `Up` / `Down` | Navigate Window list |
| `Enter` | Switch to selected Window (closes panel) |
| `k` | Kill selected Window's agent |
| `r` | Restart selected Window's agent |
| `p` | Switch to PR dashboard view |
| `i` | Switch to Issue/SPEC list view |
| `n` | New agent launch dialog |
| `s` | New shell Window |
| `Escape` / `Ctrl+G` | Close management panel |

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

Each PTY pane has a scrollback buffer, backed by gwt-core's `terminal/scrollback.rs` (file-based persistence at `~/.gwt/terminals/{id}.log`). In the TUI:

- **Mouse wheel / trackpad scroll is always active** — no special mode needed to scroll. Scrolling up freezes the viewport; new PTY output continues buffering but the viewport stays at the scrolled position.
- `Ctrl+G, PgUp` also enters scroll mode for keyboard-only navigation.
- When scrolled up, a "scroll indicator" shows lines from bottom (e.g., `[+142 lines]`).
- Scrolling to the very bottom (or pressing `q`/`Escape` in keyboard scroll mode) returns to live output tracking.
- Scrollback is preserved across Window switches.
- AI summary generation reads from the scrollback buffer.

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
