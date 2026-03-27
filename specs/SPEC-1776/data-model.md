# Data Model: SPEC-1776 — TUI Migration

## Core State

### TuiState

Central application state for gwt-tui.

```
TuiState
  tabs: Vec<TabInfo>           -- Ordered list of open tabs
  active_tab: usize            -- Index of currently focused tab
  layout: LayoutTree           -- Split pane layout (binary tree)
  management_visible: bool     -- Management panel toggle
  management_focus: Section    -- Which management section has focus
  prefix_active: bool          -- Ctrl+G prefix key state
  prefix_timeout: Instant      -- When prefix expires (2s)
  pane_manager: PaneManager    -- gwt-core pane lifecycle (owned)
```

### TabInfo

Metadata for a single tab.

```
TabInfo
  pane_id: String              -- Maps to TerminalPane in PaneManager
  tab_type: TabType            -- Agent | Shell
  label: String                -- Display name in tab bar
  branch: Option<String>       -- Git branch (agents only)
  spec_id: Option<String>      -- Associated SPEC (agents only)
  agent_type: Option<String>   -- "claude" | "codex" | "gemini"
  color: AgentColor            -- Tab color indicator
```

### LayoutTree

Binary tree for split pane layout.

```
LayoutNode
  Leaf { pane_id: String }
  Split { direction: H|V, ratio: f64, first: Node, second: Node }
```

### ManagementState

State for the management panel overlay.

```
ManagementState
  selected_agent: usize        -- Cursor position in agent list
  active_section: Section      -- AgentList | Detail | PrDashboard | Issues
  pr_cache: Vec<PrStatus>      -- Cached PR statuses
  issue_cache: Vec<IssueInfo>  -- Cached Issue list
  summary_cache: Map<String, String>  -- pane_id -> AI summary
```

## Lifecycle

```
App start
  -> Create PaneManager
  -> Open default shell tab (or restore previous session)
  -> Enter event loop

Event loop (100ms tick)
  -> Poll crossterm events (key, mouse, resize)
  -> Poll PTY output from all panes
  -> Render active frame

Tab create (agent)
  -> PaneManager::launch_agent() with BuiltinLaunchConfig
  -> Auto-create worktree if branch specified
  -> Add TabInfo to tabs vector
  -> Switch to new tab

Tab create (shell)
  -> PaneManager::spawn_shell()
  -> Add TabInfo to tabs vector

Tab close
  -> PaneManager::close_pane()
  -> Worktree cleanup (if agent tab with auto-created worktree)
  -> Remove TabInfo
  -> Adjust active_tab index

App shutdown
  -> PaneManager::kill_all()
  -> Save session state for restore
```
