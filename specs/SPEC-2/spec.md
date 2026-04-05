# Workspace Shell -- Tabs, Split Grid, Management Panel, Keybindings

## Background

gwt-tui's workspace shell manages terminal sessions (shell and agent) with two display modes: tab view (one session visible at a time) and split window (equal grid showing multiple sessions simultaneously). A management panel containing Branches, Issues, PRs, Profiles, Git View, Versions, Settings, and Logs tabs can be toggled on/off. All navigation uses a Ctrl+G prefix key system with a 2-second timeout. Session state is persisted for restore on restart. The application follows an Elm Architecture pattern (Model/Message/Update/View).

## User Stories

### US-1: Switch Between Tab and Split Window Modes (P0) -- PARTIALLY IMPLEMENTED

As a developer, I want to toggle between tab view and split grid view so that I can focus on one session or monitor multiple sessions simultaneously.

**Acceptance Scenarios**

1. Given I am in tab mode with 4 sessions, when I press Ctrl+G,z, then all 4 sessions display in a 2x2 grid.
2. Given I am in split mode, when I press Ctrl+G,z, then only the active session is displayed full-size.
3. Given I am in split mode with 3 sessions, when a session is added, then the grid layout adjusts to accommodate the new session.

### US-2: Create, Switch, and Close Shell and Agent Sessions (P0) -- IMPLEMENTED

As a developer, I want to create, switch between, and close terminal sessions so that I can manage multiple workstreams.

**Acceptance Scenarios**

1. Given any state, when I press Ctrl+G,c, then a new shell session is created and becomes active.
2. Given multiple sessions exist, when I press Ctrl+G,], then the next session becomes active.
3. Given multiple sessions exist, when I press Ctrl+G,[, then the previous session becomes active.
4. Given multiple sessions exist, when I press Ctrl+G,3, then session 3 becomes active directly.
5. Given a session is active, when I press Ctrl+G,x, then the session is closed (with unsaved changes warning if applicable).

### US-3: Toggle Management Panel Visibility (P0) -- IMPLEMENTED

As a developer, I want to toggle the management panel so that I can access project information without leaving my terminal sessions.

**Acceptance Scenarios**

1. Given the management panel is hidden, when I press Ctrl+G,g, then the panel appears on the left side.
2. Given the management panel is visible, when I press Ctrl+G,g, then the panel hides and terminal sessions reclaim the space.
3. Given the management panel is visible, when I switch sessions, then the panel remains visible.

### US-4: Navigate Management Tabs (P0) -- IMPLEMENTED

As a developer, I want to navigate between management tabs so that I can access different project information views.

**Acceptance Scenarios**

1. Given the management panel is visible, when I use tab navigation keys, then I can cycle through all 8 tabs: Branches, Issues, PRs, Profiles, Git View, Versions, Settings, Logs.
2. Given I am on the Branches tab, when I navigate to Settings, then the Settings content loads and displays.
3. Given I switch management tabs, when I return to a previous tab, then its scroll position and selection state are preserved.

### US-5: View Help Overlay Showing All Keybindings (P1) -- IMPLEMENTED

As a developer, I want to see a help overlay with all available keybindings so that I can discover and remember shortcuts.

**Acceptance Scenarios**

1. Given any state, when I press Ctrl+G,?, then a help overlay appears listing all keybindings.
2. Given the help overlay is visible, when I press Escape or Ctrl+G,?, then the overlay closes.
3. Given keybindings are added or changed in code, when the help overlay renders, then it auto-collects current keybindings from code definitions (no manual sync needed).

### US-6: Restore Session Layout on Restart (P1) -- IMPLEMENTED

As a developer, I want my session layout to be restored when I restart gwt so that I can resume work quickly.

**Acceptance Scenarios**

1. Given I have 3 sessions open with specific working directories, when I quit and restart gwt, then 3 sessions are recreated in the same directories.
2. Given I was in split mode when I quit, when I restart, then split mode is restored.
3. Given a session's working directory no longer exists, when restoring, then that session falls back to the home directory with a warning.

### US-7: Navigate Keybindings with Ctrl+G Prefix (P0) -- IMPLEMENTED

As a developer, I want all navigation keybindings to use a consistent Ctrl+G prefix so that they do not conflict with terminal applications running inside sessions.

**Acceptance Scenarios**

1. Given I press Ctrl+G, when I wait without pressing another key for 2 seconds, then the prefix times out and is cancelled.
2. Given I press Ctrl+G, when I press an unbound key, then no action is taken and the prefix is consumed.
3. Given a terminal app is running inside a session, when I type Ctrl+G followed by a bound key, then gwt intercepts it (not the terminal app).

## Edge Cases

- Ctrl+G pressed while a modal dialog (e.g., unsaved changes warning) is active.
- Split grid with 1 session (should display as full-size, not 1x1 grid).
- Closing the last remaining session (should exit gwt or create a new default session).
- Session restore when the persisted state file is corrupted or incompatible.
- Management panel toggle during an active text selection in the terminal.
- Rapid Ctrl+G prefix followed by key press (within a few milliseconds).
- Split mode with more than 9 sessions (grid layout scaling).

## Functional Requirements

- **FR-001**: Tab mode shows single active session with session tabs in the Block title (active session yellow/bold, inactive gray, separated by │).
- **FR-001a**: When the session pane is too narrow to fit the full session tab strip in the pane title, the title collapses to the active session only so the current workstream stays legible; extra-wide panes restore the full strip.
- **FR-001b**: That compact session title still preserves multi-session context by showing the active session position as `n/N`, so standard-width workspaces do not lose track of how many sessions remain open when the strip collapses.
- **FR-002**: Split mode shows an equal grid of all sessions (e.g., 2x2 for 4 sessions, 2x3 for 5-6).
- **FR-002a**: Split/grid mode pane titles preserve session identity by showing each pane's stable `n:` shortcut position plus the session-type icon alongside the session name, so the old-TUI numeric muscle memory still applies when multiple panes are visible.
- **FR-003**: Toggle between tab and split with Ctrl+G,z.
- **FR-004**: Ctrl+G prefix key system with a 2-second timeout; state machine in `keybind.rs`.
- **FR-005**: Management panel toggles visibility with Ctrl+G,g.
- **FR-005a**: Ctrl+G,g treats the management panel as a supplemental surface: showing it does not steal terminal focus, and hiding it normalizes focus back to Terminal so the main layer never advertises stale management-only hints.
- **FR-006**: 8 management tabs: Branches, Issues, PRs, Profiles, Git View, Versions, Settings, Logs. (SPECs tab removed — SPECs are shown in Branch Detail view.)
- **FR-006e**: Global management-tab shortcuts (`Ctrl+G,b/i/s/...`) also behave like supplemental surfaces when invoked from Terminal: they open the requested tab without stealing terminal focus, while management-local tab switches still land on `TabContent`.
- **FR-006f**: When the user has explicitly focused a management list/pane, `Esc` still behaves like a supplemental-surface escape hatch: if no search/detail/edit flow claims it and no warn notification is pending, focus returns to `Terminal`; warn dismissal keeps priority when a warn toast is present.
- **FR-006g**: Branch Detail content stays chrome-light: once the pane border already shows the active section and selected branch context, the inner detail renderer must not repeat nested section titles such as `Overview`, `SPECs`, `Git Status`, or `Sessions`.
- **FR-006g**: Management focus cycling is tab-aware: `Branches` keeps the old three-surface cycle (`Terminal <-> TabContent <-> BranchDetail`), while every other management tab only cycles between `Terminal` and `TabContent` so focus never lands on a non-existent detail pane.
- **FR-006a**: Branch Detail view: Branches tab is split vertically — top 50% branch list, bottom 50% detail of selected branch (always visible). Cursor movement in the list updates the visible detail from cached branch-detail data without waiting on synchronous external commands. Branch detail data is prefetched asynchronously at startup and refreshed asynchronously via `r`. Sections:
  - **Overview**: Branch name, head status, worktree path, linked Issues, PR status
  - **SPECs**: SPEC list from the branch's worktree `specs/` directory (worktree-only)
  - **Git Status**: Staged/unstaged/untracked files, recent commits
  - **Sessions**: Active agent/shell sessions on this branch, rendered as a typed session list with an active-session marker and a current selection marker
  - In the Sessions section, Up/Down cycles branch-scoped session rows and Enter focuses the selected session in the terminal pane
  - Outside the Sessions section, old-TUI direct branch actions remain available from the detail pane only when the selected branch has a worktree: Shift+Enter opens a shell on the selected worktree branch and Ctrl+C opens the delete-worktree confirmation
  - Old-TUI local mnemonics remain available from the detail pane as well: `m` toggles the Branches view mode, `v` jumps to Git View, `f` starts Branches search and returns focus to the list, and `?` / `h` opens the help overlay
  - The Branch Detail pane title keeps the selected branch name visible alongside the section tabs so context is preserved after focus moves off the top list
  - Esc in Branch Detail returns focus to the branch list without clearing the selected branch, active section, or session-row selection
  - That Branch Detail `Esc` back action is consumed locally; it does not dismiss warn notifications until a later unclaimed `Esc` occurs from the list surface
  - Left/Right cycles between sections within the detail view.
  - Enter in the detail pane directly launches agent (no action modal).
  - PR creation and branch deletion are NOT included (use CLI).
- **FR-006b**: Branch line display: name + worktree icon (U+25CF/U+25CB) + HEAD indicator. No category headers.
- **FR-006c**: Management chrome: there is no standalone header banner above the management panes. Context is carried by the pane titles themselves so the management pane keeps its full vertical space for list/detail content.
- **FR-006d**: Branch list: Enter=Wizard, Shift+Enter=Shell, Space=select, Ctrl+C=delete
- **FR-006h**: When the management pane is too narrow to fit the full tab strip in the pane title, the title collapses to the active management tab only so the current surface stays legible instead of showing a truncated strip.
- **FR-007**: New shell session created via Ctrl+G,c.
- **FR-008**: Close session via Ctrl+G,x with unsaved changes warning when applicable.
- **FR-009**: Session navigation: Ctrl+G,] (next), Ctrl+G,[ (prev), Ctrl+G,1-9 (direct).
- **FR-010**: Help overlay via Ctrl+G,? auto-collects keybindings from code definitions.
- **FR-011**: Session metadata persisted to `~/.gwt/sessions/` in TOML format.
- **FR-012**: Restore session layout on gwt restart (best-effort: working directories, display mode, active tab).
- **FR-013**: Status bar shows current session info, branch name, and agent type.
- **FR-013a**: The bottom status line keeps the old-TUI always-on context model: session summary and branch/agent context stay visible even while focus changes.
- **FR-013b**: Context-sensitive keybind hints remain visible in the status bar instead of replacing the status context entirely.
- **FR-013c**: At terminal widths `<= 80` and while no notification is occupying the footer, Terminal-focused status-bar hints use a compact grouped notation (`Ctrl+G:b/i/s g c []/1-9 z ?`) and keep `Tab:focus` / `^C×2` visible without truncating the footer.
- **FR-013d**: At terminal widths `<= 80` and while no notification is occupying the footer, management and Branch Detail footers also switch to compact hint notation so the pane-local affordances remain visible instead of truncating behind the status context.
- **FR-013e**: Non-Branches management footer hints are mode-aware instead of generic: tabs without sub-tabs omit `Ctrl+←→:sub-tab`, detail drill-downs advertise `Esc:back`, and form/edit modes advertise `Esc:cancel`.
- **FR-013f**: Non-Branches management footer hints are also action-aware: each tab advertises only its real primary action surface instead of a generic `Enter:action`, such as `Issues` list showing `/:search` and `Enter:detail`, `Git View` showing `Enter:expand`, `Versions` showing refresh-only, and `PR Dashboard` detail showing `Enter:close`.
- **FR-014**: Management panel width is adjustable or uses a sensible default proportion. The current default split is responsive: wide terminals (`>=120 cols`) use `40% management / 60% session`, while standard or narrower terminals fall back to `50% / 50%` so management chrome remains legible.
- **FR-014a**: Session PTY geometry is initialized from the actual visible session-pane size at startup. The default shell must not stay on the stale `80x24` model default until a later terminal resize event arrives.
- **FR-015**: Focus system: Branches exposes 3 focusable panes (`TabContent`, `BranchDetail`, `Terminal`) cycled with `Tab`/`Shift+Tab`, while every other management tab exposes only `TabContent` and `Terminal`. Focused pane has blue (Cyan) border, unfocused has white (Gray) border. Reverse focus cycling must work whether the terminal reports `Shift+Tab` as `BackTab` or as `Tab` with the Shift modifier.
- **FR-016**: Arrow keys (↑↓←→) replace vim-style j/k/h/l for all navigation. No vim keybindings.
- **FR-017**: Overlays (Wizard, Confirm, Error) capture all keyboard input when visible, preventing focus pane from receiving keys.

## Non-Functional Requirements

- **NFR-001**: Startup to interactive state under 500ms.
- **NFR-002**: Session switch completes under 50ms (no visible delay).
- **NFR-003**: Ctrl+G prefix state machine handles rapid input without missed keys.
- **NFR-004**: Split grid layout recalculates within one frame on session add/remove.
- **NFR-005**: Session persistence file size remains under 100KB for typical usage.
- **NFR-006**: Branch list selection changes complete without synchronous `git` / Docker / filesystem detail reloads on the input path.
- **NFR-007**: Startup PTY geometry matches the visible session pane without requiring a manual terminal resize.

## Implementation Details

### Focus System

Branches uses a 3-surface cycle:

```
Tab →  Tab Content (list) → Branch Detail → Terminal → ...
```

All other management tabs use a 2-surface cycle:

```
Tab →  Tab Content (list) → Terminal → ...
```

- Focused pane: **blue** border (`Color::Cyan`)
- Unfocused pane: **white** border (`Color::Gray`)
- Reverse focus cycling accepts both `BackTab` and `Shift+Tab` key encodings
- Management tabs are rendered in the Block title of the management panel (Left/Right switches tabs within TabContent focus)
- Session tabs are rendered in the Block title of the terminal content area (active session highlighted yellow/bold, inactive gray)
- Ctrl+G,g toggles management panel visibility (same as before)
- Overlays (Wizard, Confirm, Error) capture all input when visible

### Global Keybindings (work regardless of focus)

| Keybinding | Action |
|------------|--------|
| `Tab` | Move focus to next pane |
| `Shift+Tab` | Move focus to previous pane |
| `Ctrl+G, g` | Toggle management panel visibility |
| `Ctrl+G, c` | New shell session |
| `Ctrl+G, q` | Quit |
| `Ctrl+G, ?` | Show help overlay |
| `Ctrl+G, v` | Voice input (start recording) |
| `Ctrl+G, p` | Paste file paths from clipboard |
| `Ctrl+G, b` | Switch to Branches tab |
| `Ctrl+G, s` | Switch to Settings tab |
| `Ctrl+G, i` | Switch to Issues tab |
| `Ctrl+G, ]` | Next session |
| `Ctrl+G, [` | Previous session |
| `Ctrl+G, 1-9` | Switch to session N |
| `Ctrl+G, z` | Toggle Tab/Grid layout |
| `Ctrl+G, x` | Close current session |
| `Ctrl+C, Ctrl+C` | Quit (double-tap, 500ms window) |

### Management Tab Header (in Block title)

Management tabs are displayed in the Block title of the management panel area. Tab switching (Left/Right) is available when TabContent has focus. There is no separate TabHeader focus pane.

### Focus: Tab Content (list area)

| Keybinding | Action |
|------------|--------|
| `↑` / `↓` | Navigate list items |
| `Enter` | Select / toggle detail view |
| `Esc` | Cancel search / close detail / return focus to terminal |
| `/` | Start search (Branches, Issues) |
| `r` | Refresh data |
| `s` | Toggle sort mode (Branches only) |
| `n` | New / Add (Profiles only) |
| `e` | Edit (Profiles only) |
| `d` | Delete (Profiles only) |
| `Space` | Toggle boolean (Settings only) |
| `Ctrl+←` / `Ctrl+→` | Switch sub-tab (Settings categories, Logs filters) |

### Focus: Branch Detail

| Keybinding | Action |
|------------|--------|
| `←` / `→` | Switch detail section (Overview/SPECs/Git/Sessions/Actions) |
| `↑` / `↓` | Navigate within section (Actions list) |
| `Enter` | Launch agent for the selected branch, or focus the selected session inside `Sessions` |
| `Shift+Enter` | Open shell for the selected branch when it has a worktree (outside `Sessions`) |
| `Ctrl+C` | Open delete-worktree confirmation when the selected branch has a worktree (outside `Sessions`) |
| `m` | Toggle the Branches view mode without leaving Branch Detail |
| `v` | Jump directly to Git View |
| `f` | Start Branches search and return focus to the list |
| `?` / `h` | Open the help overlay |
| `Esc` | Return focus to the Branches list while preserving detail context |

### Focus: Terminal

| Keybinding | Action |
|------------|--------|
| All keys | Forwarded to PTY (except Ctrl+G prefix and Tab) |

### Overlay: Wizard (captures all input)

| Keybinding | Action |
|------------|--------|
| `↑` / `↓` | Navigate options |
| `Enter` | Select / advance step |
| `Esc` | Go back / cancel |
| Character keys | Text input (branch name, URL, etc.) |
| `Backspace` | Delete character |

### Overlay: Confirm Dialog

| Keybinding | Action |
|------------|--------|
| `←` / `→` | Toggle Yes/No |
| `Enter` | Accept |
| `Esc` | Cancel |

### Overlay: Error

| Keybinding | Action |
|------------|--------|
| `Enter` / `Esc` | Dismiss error |

### Ctrl+G Prefix State Machine

```
Idle ─[Ctrl+G]─→ PrefixActive
PrefixActive ─[bound key]─→ Execute action → Idle
PrefixActive ─[Ctrl+G]─→ Toggle management → Idle
PrefixActive ─[Esc or 2s timeout]─→ Cancel → Idle
PrefixActive ─[unbound key]─→ Ignore → Idle
```

### Session Persistence Schema

Persisted to `~/.gwt/sessions/{base64_worktree_path}.toml`:

```toml
display_mode = "tab"  # "tab" | "grid"
management_visible = true
active_management_tab = "branches"
active_session = 0

[[sessions]]
pane_id = "uuid"
tab_type = "shell"  # "shell" | "agent"
working_dir = "/path/to/dir"
branch = "feature/foo"
agent_id = "claude"  # only for agent type
```

## Success Criteria

- **SC-001**: Tab and split mode toggle works correctly for 1-9 sessions.
- **SC-002**: All Ctrl+G keybindings are functional and do not conflict with terminal apps.
- **SC-003**: Help overlay auto-collects keybindings and displays them correctly.
- **SC-004**: Session restore recreates previous layout after quit and restart.
- **SC-005**: Management panel tabs all render content and preserve state between switches.
- **SC-006**: Status bar updates within one frame of session or branch change.
- **SC-007**: `Shift+Tab` moves focus backward on both Branches and non-Branches management tabs even when the terminal emits `Tab` with the Shift modifier instead of `BackTab`.
- **SC-008**: On startup, the default shell PTY sees the same row/column geometry as the visible session pane before any manual terminal resize occurs.
