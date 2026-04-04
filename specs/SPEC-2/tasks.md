# Workspace Shell — Tasks

## Phase 0: Implementation Details Documentation

- [x] T-W01 Document complete keybinding map (32 bindings) in spec.md
- [x] T-W02 Document Ctrl+G prefix state machine in spec.md
- [x] T-W03 Document session persistence TOML schema in spec.md
- [x] T-W04 Create data-model.md with all state structs
- [x] T-W05 Create research.md with architecture decisions
- [x] T-W06 Create quickstart.md with validation flow

## Phase 1: Help Overlay Auto-Collection

- [x] T001 [P] Write RED test: help overlay output contains all registered keybindings.
- [x] T002 [P] Write RED test: help overlay does not contain unregistered key sequences.
- [x] T003 Define keybinding registry data structure (key sequence, description, category).
- [x] T004 Populate registry from existing keybind.rs match arms.
- [x] T005 Render help overlay (Ctrl+G,?) using registry data, grouped by category.
- [x] T006 Verify help overlay tests pass GREEN.

## Phase 2: Session Persistence Improvement

- [x] T007 [P] Write RED test: session save/restore round-trip preserves display mode, panel visibility, active tab.
- [x] T008 [P] Write RED test: corrupted session file triggers graceful fallback with warning.
- [x] T009 [P] Write RED test: missing session directory is auto-created on save.
- [x] T010 Extend session persistence TOML schema with display_mode, panel_visible, active_management_tab fields.
- [x] T011 Implement save logic for new fields.
- [x] T012 Implement restore logic with graceful fallback for missing/corrupted fields.
- [x] T013 Verify session persistence tests pass GREEN.

## Phase 3: Git View Tab

- [x] T014 [P] Write RED test: Git View tab renders recent commit list from git log.
- [x] T015 [P] Write RED test: Git View tab handles empty repository gracefully.
- [x] T016 Implement Git View management tab component.
- [x] T017 Wire Git View into management panel tab navigation.
- [x] T018 Verify Git View tests pass GREEN.

## Phase 4: Branch Detail View (SPECs Tab Removal)

### 4.1: Remove SPECs Tab

- [x] T022 Remove ManagementTab::Specs variant from model.rs (9→8 tabs)
- [x] T023 Remove specs tab routing from app.rs (route_key_to_management, render_management_tab_content)
- [x] T024 Update Ctrl+G,s keybind to switch to Settings instead of SPECs
- [x] T025 Update E2E tests for 8-tab layout

### 4.2: Branch Detail Split Layout

- [x] T026 [P] Write RED test: Branches tab renders top 50% list + bottom 50% detail
- [x] T027 Modify branches.rs render() to split area vertically (50/50)
- [x] T028 Create branch_detail rendering function showing selected branch info
- [x] T029 Wire cursor movement to update detail content

### 4.3: Detail Sections

- [x] T030 [P] Write RED test: detail section cycles through Overview/SPECs/GitStatus/Sessions with Tab
- [x] T031 Implement Overview section: branch name, head, worktree path, category
- [x] T032 Implement SPECs section: placeholder with branch name
- [x] T033 Implement Git Status section: placeholder with branch name
- [x] T034 Implement Sessions section: placeholder "No active sessions"
- [x] T035 ~~Actions section removed~~ — replaced by action modal overlay (Enter in detail pane)

### 4.4: Actions Implementation (via Action Modal)

- [x] T036 [P] Write RED test: Agent launch from action modal sets pending flag
- [x] T037 [P] Write RED test: Shell launch sets pending flag
- [x] T038 [P] Write RED test: Worktree delete sets pending flag
- [x] T039 Implement agent launch action: set pending_launch_agent flag
- [x] T040 Implement shell launch action: set pending_open_shell flag
- [x] T041 Implement worktree delete action: set pending_delete_worktree flag

### 4.5: Integration

- [x] T042 Add detail_section, action_modal_visible/selected, pending flags to BranchesState
- [x] T043 Add ActionModal message variants to BranchesMessage
- [x] T044 Route Left/Right to detail section cycling in route_key_to_branch_detail
- [x] T045 Route Enter in detail pane to open action modal; modal keys routed via route_overlay_key
- [x] T046 Write 15 tests for branch detail state transitions and rendering
- [x] T047 Update all E2E snapshot tests

## Phase 5: Focus System + Keybinding Rework

### 5.1: Focus State

- [x] T053 Add FocusPane enum to model.rs: TabContent, BranchDetail, Terminal (3 panes, no TabHeader)
- [x] T054 Add active_focus: FocusPane field to Model
- [x] T055 Write RED test: Tab cycles through 3 focus panes in order
- [x] T056 Write RED test: Shift+Tab cycles in reverse
- [x] T057 Implement Tab/Shift+Tab focus cycling in app.rs update()
- [x] T058 Implement focus-aware border colors: Cyan (focused) / Gray (unfocused)
- [x] T059 Write render test: focused pane has Cyan border

### 5.2: Replace j/k with Arrow Keys

- [x] T060 Replace KeyCode::Char('j') with KeyCode::Down in all screens
- [x] T061 Replace KeyCode::Char('k') with KeyCode::Up in all screens
- [x] T062 Update route_key_to_management to use arrow keys only
- [x] T063 Update all screen tests to use arrow keys
- [x] T064 Update E2E tests to use arrow keys

### 5.3: Focus-Aware Key Routing

- [x] T065 In app.rs: route keys based on active_focus instead of active_layer
- [x] T066 TabHeader focus: Left/Right switch tabs, Enter moves focus to TabContent
- [x] T067 TabContent focus: ↑↓ navigate list, Enter select, / search, r refresh
- [x] T068 BranchDetail focus: ←→ switch sections, ↑↓ navigate actions, Enter execute
- [x] T069 Terminal focus: forward all keys to PTY (except Tab and Ctrl+G prefix)
- [x] T070 Write 10+ tests for focus-aware key routing

### 5.4: Update Overlay Key Routing

- [x] T071 Wizard: ↑↓ navigate, Enter select, Esc back, char input
- [x] T072 Confirm: ←→ toggle, Enter accept, Esc cancel
- [x] T073 Error: Enter/Esc dismiss

### 5.5: Sub-Tab Switching with Ctrl+Arrow Keys

- [x] T079 Add FilterLevel::prev() to logs.rs for reverse filter cycling
- [x] T080 Exclude Ctrl-modified arrow keys from management tab switching in route_key_to_management
- [x] T081 Add Ctrl+Left/Right for Settings category switching (PrevCategory/NextCategory)
- [x] T082 Add Ctrl+Left/Right for Logs filter cycling (prev/next filter level)
- [x] T083 Add tests for FilterLevel::prev() and Ctrl+Arrow key routing

## Phase 6: Regression and Polish

- [x] T074 Run full existing test suite and verify no regressions.
- [x] T075 Run `cargo clippy` and `cargo fmt` on all changed files.
- [x] T076 Verify coverage >= 90%.
- [x] T077 Confirm the referenced `simplify` skill is not exposed in the current session and retire this obsolete process task.
- [x] T078 Update SPEC-2 metadata phase to Done.

## Note: Unified Tab Display Pattern

All tab displays across the TUI are now unified to use the Block title pattern
(tab names rendered in the border title, active=yellow/bold, inactive=gray,
separated by │). This includes:

- Management tabs (app.rs) - already Block title
- Session tabs (app.rs) - already Block title
- Branch detail section tabs (branches.rs) - converted from Tabs::new
- SPEC detail section tabs (specs.rs) - converted from Tabs::new
- Settings category tabs (settings.rs) - converted from Tabs::new
- Log filter tabs (logs.rs) - converted from Tabs::new
- Session tab bar widget (widgets/tab_bar.rs) - converted from Tabs::new

A shared `build_tab_title()` utility in `screens/mod.rs` is available for all
screens to use.

## Phase 7: TUI Operation Flow Overhaul

- [x] T084-T103 All tasks completed

## Phase 8: Branch-First UX Restoration

- [x] T104 [P] Write RED test: branch list renders without category headers or locality badges and shows HEAD/worktree indicators in-line.
- [x] T105 [P] Write RED test: Branches tab routes `Enter`, `Shift+Enter`, `Space`, and `Ctrl+C` to wizard, shell, detail focus, and worktree delete respectively.
- [x] T106 Update `branches.rs` list rendering to the old-TUI style branch line format (`name + worktree indicator + HEAD indicator`) with no category headers.
- [x] T107 Update `app.rs` branch key routing and footer hints to restore branch-first primary actions without regressing existing focus-aware behavior.
- [x] T108 Verify focused branch UX tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 9: Branch Mnemonic Restoration

- [x] T109 [P] Write RED test: Branches tab restores `m` for view-mode cycling and `v` for direct Git View navigation.
- [x] T110 [P] Write RED test: Branches tab restores `f` as a search alias and `?` / `h` as local help entry points.
- [x] T111 Update `app.rs` Branches key routing to use old-TUI branch-local mnemonics without regressing search or overlay behavior.
- [x] T112 Update branch-specific footer hints and snapshots so the restored mnemonics are discoverable.
- [x] T113 Verify focused mnemonic tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 10: Branch Detail Sessions Restoration

- [x] T114 [P] Write RED test: `Sessions` detail renders branch-scoped session summaries with session type and active marker.
- [x] T115 [P] Write RED test: session summary extraction only returns sessions that belong to the selected branch and flags the active session.
- [x] T116 Add a lightweight branch-session summary helper in `app.rs` without touching `model.rs`.
- [x] T117 Replace the count-only `Sessions` detail placeholder in `branches.rs` with the typed session list while preserving the empty state.
- [x] T118 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 11: Branch Detail Session Focus Actions

- [x] T119 [P] Write RED test: `Sessions` detail renders a selection marker for the currently selected branch session row.
- [x] T120 [P] Write RED test: `Up/Down` inside the `Sessions` detail section cycles the session-row selection.
- [x] T121 [P] Write RED test: `Enter` inside the `Sessions` detail section activates the selected session and focuses the terminal pane.
- [x] T122 Add lightweight session-row selection state and routing in `app.rs` / `branches.rs`.
- [x] T123 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 12: Status Bar Restoration

- [x] T124 [P] Write RED test: shell sessions render branch context and `type: Shell` in the status bar.
- [x] T125 [P] Write RED test: agent sessions render agent type and selected-branch context in the status bar.
- [x] T126 [P] Write RED test: Branches focus still shows the branch-first keybind hints when the footer is rendered through the status-bar widget.
- [x] T127 Restore the bottom status bar in `app.rs` / `widgets/status_bar.rs` so it carries session context, branch/agent metadata, notifications, and hints together.
- [x] T128 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 13: Branch Detail Direct Actions

- [x] T129 [P] Write RED test: `Shift+Enter` in Branch Detail opens a shell for the selected branch outside the `Sessions` section.
- [x] T130 [P] Write RED test: `Ctrl+C` in Branch Detail opens the delete-worktree confirmation outside the `Sessions` section.
- [x] T131 [P] Write RED test: Branch Detail footer hints become section-sensitive (`Sessions` shows focus-session semantics, Overview shows direct branch actions).
- [x] T132 Update `app.rs` Branch Detail routing and footer hints to restore old-TUI direct actions without regressing the `Sessions` section handoff.
- [x] T133 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 14: Branch Detail Title Context

- [x] T134 [P] Write RED test: Branch Detail pane title includes the selected branch name while preserving section-tab highlighting.
- [x] T135 [P] Write RED test: Branch Detail pane title falls back gracefully when no branch is selected.
- [x] T136 Update `app.rs` Branch Detail pane chrome to append selected branch context to the title without touching shared tab-title utilities.
- [x] T137 Refresh `SPEC-2` artifacts to describe the restored title-context contract.
- [x] T138 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 15: Branch Detail Escape Back

- [x] T139 [P] Write RED test: `Esc` in Branch Detail returns focus to the Branches list.
- [x] T140 [P] Write RED test: `Esc` in Branch Detail preserves the selected branch, active section, and session-row selection.
- [x] T141 Update `app.rs` Branch Detail routing so `Esc` returns to `TabContent` without mutating detail state.
- [x] T142 Refresh `SPEC-2` artifacts to describe the restored `Esc:back` contract.
- [x] T143 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 16: Focus Border Color Parity

- [x] T144 [P] Write RED test: focused pane borders render with `Color::Cyan`.
- [x] T145 [P] Write RED test: unfocused pane borders render with `Color::Gray`.
- [x] T146 Update `app.rs` pane chrome so `pane_block()` uses the documented `Cyan/Gray` focus colors.
- [x] T147 Refresh `SPEC-2` artifacts to describe the restored focus-border parity.
- [x] T148 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 17: Worktree-Aware Branch Detail Direct Actions

- [x] T149 [P] Write RED test: no-worktree Branch Detail hints omit `Shift+Enter:shell` and `Ctrl+C:delete`.
- [x] T150 [P] Write RED test: `Shift+Enter` in Branch Detail does not open a shell when the selected branch has no worktree.
- [x] T151 [P] Write RED test: `Ctrl+C` in Branch Detail does not open delete confirmation when the selected branch has no worktree.
- [x] T152 Update `app.rs` Branch Detail hint/routing/pending-action handling to require a selected worktree branch for direct shell/delete actions.
- [x] T153 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 18: Management Panel Width Default

- [x] T154 [P] Write RED test: the shared management/session split helper returns a `40/60` geometry.
- [x] T155 [P] Write RED test: `active_session_content_area()` uses the session side of the `40/60` split while management is visible.
- [x] T156 Update `app.rs` layout code to use a shared `40/60` management/session split helper for both rendering and active-session geometry.
- [x] T157 Refresh `SPEC-2` artifacts to describe the restored management width default.
- [x] T158 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 19: Branch Detail Local Mnemonics

- [x] T159 [P] Write RED test: `m` in Branch Detail toggles the Branches view mode without leaving the detail pane.
- [x] T160 [P] Write RED test: `v` and `f` in Branch Detail mirror Git View and search routing, and `?` / `h` still opens help.
- [x] T161 [P] Write RED test: Branch Detail footer hints advertise the restored local mnemonics.
- [x] T162 Update `app.rs` Branch Detail routing and hint text to mirror `m=view`, `v=Git View`, `f=search`, and `?` / `h=help`.
- [x] T163 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 20: Compact Management Header Context

- [x] T164 [P] Write RED test: the management header renders the repository basename instead of the full path.
- [x] T165 [P] Write RED test: the management header includes compact active-context text for the active tab/focus.
- [x] T166 Update `app.rs` management header rendering to use compact basename/context text that fits the 40% pane more gracefully.
- [x] T167 Refresh `SPEC-2` artifacts to describe the compact management-header contract.
- [x] T168 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.
