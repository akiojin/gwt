# Workspace Shell -- Tabs, Split Grid, Management Panel, Keybindings

## Background

gwt-tui's workspace shell manages terminal sessions (shell and agent) with two display modes: tab view (one session visible at a time) and split window (equal grid showing multiple sessions simultaneously). A management panel containing Branches, SPECs, Issues, Profiles, Git View, Versions, Settings, and Logs tabs can be toggled on/off. All navigation uses a Ctrl+G prefix key system with a 2-second timeout. Session state is persisted for restore on restart. The application follows an Elm Architecture pattern (Model/Message/Update/View).

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

1. Given the management panel is visible, when I use tab navigation keys, then I can cycle through all 7 tabs: Branches, Issues, Profiles, Git View, Versions, Settings, Logs.
2. Given I am on the Branches tab, when I navigate to Settings, then the Settings content loads and displays.
3. Given I switch management tabs, when I return to a previous tab, then its scroll position and selection state are preserved.

### US-5: View Help Overlay Showing All Keybindings (P1) -- PARTIALLY IMPLEMENTED

As a developer, I want to see a help overlay with all available keybindings so that I can discover and remember shortcuts.

**Acceptance Scenarios**

1. Given any state, when I press Ctrl+G,?, then a help overlay appears listing all keybindings.
2. Given the help overlay is visible, when I press Escape or Ctrl+G,?, then the overlay closes.
3. Given keybindings are added or changed in code, when the help overlay renders, then it auto-collects current keybindings from code definitions (no manual sync needed).

### US-6: Restore Session Layout on Restart (P1) -- PARTIALLY IMPLEMENTED

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

- **FR-001**: Tab mode shows single active session with a tab bar listing all sessions.
- **FR-002**: Split mode shows an equal grid of all sessions (e.g., 2x2 for 4 sessions, 2x3 for 5-6).
- **FR-003**: Toggle between tab and split with Ctrl+G,z.
- **FR-004**: Ctrl+G prefix key system with a 2-second timeout; state machine in `keybind.rs`.
- **FR-005**: Management panel toggles visibility with Ctrl+G,g.
- **FR-006**: 7 management tabs: Branches, Issues, Profiles, Git View, Versions, Settings, Logs. (SPECs tab removed — SPECs are shown in Branch Detail view.)
- **FR-006a**: Branch Detail view: Branches tab is split vertically — top 50% branch list, bottom 50% detail of selected branch (always visible). Cursor movement in the list updates the detail. Sections:
  - **Overview**: Branch name, head status, worktree path, linked Issues, PR status
  - **SPECs**: SPEC list from the branch's worktree `specs/` directory (worktree-only)
  - **Git Status**: Staged/unstaged/untracked files, recent commits
  - **Sessions**: Active agent/shell sessions on this branch
  - **Actions**: Launch agent (agent select only, no full wizard), Open shell (in worktree cwd), Delete worktree
  - Tab key cycles between sections within the detail view.
  - Agent launch: shows agent selection list only (branch already determined). No full wizard.
  - Shell: opens in the branch's worktree directory. Requires worktree to exist.
  - Worktree creation is automatic (on agent launch). Manual creation not available. Deletion available with confirmation.
  - PR creation and branch deletion are NOT included (use CLI).
- **FR-007**: New shell session created via Ctrl+G,c.
- **FR-008**: Close session via Ctrl+G,x with unsaved changes warning when applicable.
- **FR-009**: Session navigation: Ctrl+G,] (next), Ctrl+G,[ (prev), Ctrl+G,1-9 (direct).
- **FR-010**: Help overlay via Ctrl+G,? auto-collects keybindings from code definitions.
- **FR-011**: Session metadata persisted to `~/.gwt/sessions/` in TOML format.
- **FR-012**: Restore session layout on gwt restart (best-effort: working directories, display mode, active tab).
- **FR-013**: Status bar shows current session info, branch name, and agent type.
- **FR-014**: Management panel width is adjustable or uses a sensible default proportion.
- **FR-015**: Focus system: 4 focusable panes cycled with Tab/Shift+Tab. Focused pane has blue (Cyan) border, unfocused has white (Gray) border.
- **FR-016**: Arrow keys (↑↓←→) replace vim-style j/k/h/l for all navigation. No vim keybindings.
- **FR-017**: Overlays (Wizard, Confirm, Error) capture all keyboard input when visible, preventing focus pane from receiving keys.

## Non-Functional Requirements

- **NFR-001**: Startup to interactive state under 500ms.
- **NFR-002**: Session switch completes under 50ms (no visible delay).
- **NFR-003**: Ctrl+G prefix state machine handles rapid input without missed keys.
- **NFR-004**: Split grid layout recalculates within one frame on session add/remove.
- **NFR-005**: Session persistence file size remains under 100KB for typical usage.

## Implementation Details

### Focus System

4 focusable panes, cycled with Tab / Shift+Tab:

```
Tab →  Management Tab Header → Tab Content (list) → Branch Detail → Terminal → ...
```

- Focused pane: **blue** border (`Color::Cyan`)
- Unfocused pane: **white** border (`Color::Gray`)
- Ctrl+G,g toggles management panel visibility (same as before)
- Overlays (Wizard, Confirm, Error) capture all input when visible

### Global Keybindings (work regardless of focus)

| Keybinding | Action |
|------------|--------|
| `Tab` | Move focus to next pane |
| `Shift+Tab` | Move focus to previous pane |
| `Ctrl+G, g` | Toggle management panel visibility |
| `Ctrl+G, c` | New shell session |
| `Ctrl+G, n` | Open agent launch wizard |
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

### Focus: Management Tab Header

| Keybinding | Action |
|------------|--------|
| `←` / `→` | Switch management tab |
| `Enter` | Activate selected tab (move focus to Tab Content) |

### Focus: Tab Content (list area)

| Keybinding | Action |
|------------|--------|
| `↑` / `↓` | Navigate list items |
| `Enter` | Select / toggle detail view |
| `Esc` | Cancel search / close detail |
| `/` | Start search (Branches, Issues) |
| `r` | Refresh data |
| `s` | Toggle sort mode (Branches only) |
| `n` | New / Add (Profiles only) |
| `e` | Edit (Profiles only) |
| `d` | Delete (Profiles only) |
| `Space` | Toggle boolean (Settings only) |

### Focus: Branch Detail

| Keybinding | Action |
|------------|--------|
| `←` / `→` | Switch detail section (Overview/SPECs/Git/Sessions/Actions) |
| `↑` / `↓` | Navigate within section (Actions list) |
| `Enter` | Execute action (Launch Agent, Open Shell, Delete Worktree) |

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
