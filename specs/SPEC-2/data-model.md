# Data Model: SPEC-2 — Workspace Shell

## Core State

### Model (crates/gwt-tui/src/model.rs)

```
Model
  active_layer: ActiveLayer          # Initialization | Main | Management
  sessions: Vec<SessionTab>          # Shell + Agent sessions
  active_session: usize              # Index of focused session
  session_layout: SessionLayout      # Tab | Grid
  management_tab: ManagementTab      # Active management tab
  error_queue: VecDeque<String>      # Error overlay queue
  quit: bool                         # Exit flag
  repo_path: PathBuf                 # Repository root path
  terminal_size: (u16, u16)          # Terminal dimensions

  # Per-tab screen states
  branches: BranchesState
  issues: IssuesState
  git_view: GitViewState
  pr_dashboard: PrDashboardState
  profiles: ProfilesState
  settings: SettingsState
  logs: LogsState
  versions: VersionsState

  # Overlay states
  wizard: Option<WizardState>
  docker_progress: Option<DockerProgressState>
  service_select: Option<ServiceSelectState>
  port_select: Option<PortSelectState>
  confirm: ConfirmState
  voice: VoiceInputState
  pending_pty_inputs: VecDeque<PendingPtyInput>
```

### ActiveLayer

```
enum ActiveLayer {
  Initialization    # No repo detected — clone wizard
  Main              # Terminal sessions (PTY output)
  Management        # Management panel (tabs)
}
```

### ManagementTab

```
enum ManagementTab {
  Branches          # Branch list + detail (split view)
  Issues            # GitHub Issues
  Profiles          # Environment profiles
  GitView           # File diffs, commits
  Versions          # Git tags
  Settings          # 7 categories
  Logs              # Structured log viewer
}
```

Note: SPECs tab removed — SPECs are shown in Branch Detail view.

### SessionTab

```
SessionTab
  id: String                    # Unique session ID
  name: String                  # Display name
  tab_type: SessionTabType      # Shell | Agent
  vt: VtState                   # Terminal emulation state
```

### SessionLayout

```
enum SessionLayout {
  Tab    # Single session visible, tab bar shows all
  Grid   # Equal grid of all sessions (2x2, 2x3, etc.)
}
```

## Branch Detail State

### BranchesState (crates/gwt-tui/src/screens/branches.rs)

```
BranchesState
  branches: Vec<BranchItem>
  selected: usize
  sort_mode: SortMode           # Default | Name | Date
  view_mode: ViewMode           # All | Local | Remote
  search_query: String
  search_active: bool
  detail_view: bool             # Whether detail panel is showing
  detail_section: usize         # Active section (0=Overview, 1=SPECs, ...)
  detail_specs: Vec<SpecItem>   # SPECs from selected branch's worktree
  detail_files: Vec<FileEntry>  # Git status files
  detail_commits: Vec<CommitEntry>
```

### BranchItem

```
BranchItem
  name: String
  is_head: bool
  is_local: bool
  category: BranchCategory      # Main | Develop | Feature | Other
```

### Branch Detail Sections

```
enum DetailSection {
  Overview    # Branch info, worktree, PR, linked Issues
  SPECs       # SPEC list from worktree specs/
  GitStatus   # Staged/unstaged/untracked files
  Sessions    # Active sessions on this branch
  Actions     # Agent launch, shell, worktree delete
}
```

## Keybind State

### KeybindRegistry (crates/gwt-tui/src/input/keybind.rs)

```
KeybindRegistry
  prefix_state: PrefixState     # Idle | Active { since: Instant }
  bindings: Vec<Keybinding>     # All registered keybindings
  last_ctrl_c: Option<Instant>  # Double-tap quit tracking
```

### PrefixState

```
enum PrefixState {
  Idle                          # Waiting for Ctrl+G
  Active { since: Instant }     # Ctrl+G pressed, awaiting second key
}
```

## Session Persistence

### SessionState (saved to ~/.gwt/sessions/)

```
SessionState
  display_mode: String          # "tab" | "grid"
  management_visible: bool
  active_management_tab: String # Tab label
  session_count: usize
```
