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
- [x] T055 Write RED test: Ctrl+G, Tab cycles through 3 focus panes in order
- [x] T056 Write RED test: Ctrl+G, Shift+Tab cycles in reverse
- [x] T057 Implement Ctrl+G, Tab/Shift+Tab focus cycling in app.rs update()
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
- [x] T069 Terminal focus: forward all keys to PTY (except Ctrl+G prefix; Ctrl+G, Tab/Shift+Tab reserved for focus cycling)
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

This phase was an intermediate step and is superseded by Phase 30, which removes the standalone
banner in favor of pane-title chrome.

- [x] T164 [P] Write RED test: the management header renders the repository basename instead of the full path.
- [x] T165 [P] Write RED test: the management header includes compact active-context text for the active tab/focus.
- [x] T166 Update `app.rs` management header rendering to use compact basename/context text that fits the 40% pane more gracefully.
- [x] T167 Refresh `SPEC-2` artifacts to describe the intermediate compact management-header contract.
- [x] T168 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 21: Focus-Preserving Layer Toggle

- [x] T169 [P] Write RED test: toggling from Main to Management keeps `FocusPane::Terminal`.
- [x] T170 [P] Write RED test: toggling from Management to Main normalizes focus back to `FocusPane::Terminal` even if the user was in `TabContent` or `BranchDetail`.
- [x] T171 Update `app.rs` layer-toggle handling so the management panel behaves like a supplemental surface instead of stealing/sticking management focus.
- [x] T172 Refresh `SPEC-2` artifacts to describe the focus-preserving layer-toggle contract.
- [x] T173 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 22: Focus-Preserving Global Tab Switches

- [x] T174 [P] Write RED test: switching to a management tab from Terminal keeps `FocusPane::Terminal` while opening the requested tab.
- [x] T175 [P] Write RED test: switching tabs from management `TabContent`/`BranchDetail` still lands on `FocusPane::TabContent`.
- [x] T176 Update `app.rs` tab-switch handling so global management-tab shortcuts follow the supplemental-panel focus contract.
- [x] T177 Refresh `SPEC-2` artifacts to describe terminal-preserving tab switches.
- [x] T178 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 23: Esc-Back for Issues and PR Detail Views

- [x] T179 [P] Write RED test: `Esc` in `Issues` detail closes the detail view while preserving the selected row.
- [x] T180 [P] Write RED test: `Esc` in `PR Dashboard` detail closes the detail view while preserving the selected row.
- [x] T181 Update `app.rs` management routing so `Esc` closes Issues / PR detail views before falling back to warn-dismiss behavior.
- [x] T182 Refresh `SPEC-2` artifacts to describe detail-close-on-Esc parity.
- [x] T183 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 24: Esc-Back for Logs Detail View

- [x] T184 [P] Write RED test: `Esc` in `Logs` detail closes the detail view while preserving the selected row.
- [x] T185 [P] Write RED test: existing `Logs` routing (`f`, `d`, Enter`) still behaves unchanged after adding Logs detail-close-on-`Esc`.
- [x] T186 Update `app.rs` management routing so `Esc` closes `Logs` detail before falling back to warn-dismiss behavior.
- [x] T187 Refresh `SPEC-2` artifacts to describe Logs detail-close-on-Esc parity.
- [x] T188 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 25: Esc Returns from Management Pane to Terminal

- [x] T189 [P] Write RED test: in management list/pane focus, `Esc` returns focus to `Terminal` when no warn notification is pending.
- [x] T190 [P] Write RED test: when a warn notification is pending, `Esc` still dismisses the warn notification instead of changing focus.
- [x] T191 Update `app.rs` management fallback routing so unclaimed `Esc` uses the supplemental-surface contract (`warn dismiss` first, otherwise `Terminal` focus).
- [x] T192 Refresh `SPEC-2` artifacts to describe management-pane `Esc` parity.
- [x] T193 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 26: Profiles Esc Uses Supplemental Fallback in List Mode

- [x] T194 [P] Write RED test: in `Profiles` list mode, `Esc` returns focus to `Terminal` when no warn notification is pending.
- [x] T195 [P] Write RED test: in `Profiles` list mode, a visible warn notification still consumes `Esc` for dismissal first.
- [x] T196 [P] Write RED test: in `Profiles` create mode, `Esc` still cancels the form instead of changing focus.
- [x] T197 Update `app.rs` Profiles routing so `Esc` is mode-aware: list mode uses the supplemental fallback, while non-list modes keep `Cancel`.
- [x] T198 Refresh `SPEC-2` artifacts to describe Profiles list-mode `Esc` parity.
- [x] T199 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 27: Status Bar Hints Reflect Esc-to-Terminal Contract

- [x] T200 [P] Write RED test: Branches list status-bar hints include `Esc:term`.
- [x] T201 [P] Write RED test: generic management list status-bar hints include `Esc:term`.
- [x] T202 Update `app.rs` status-bar hint text so Branches list and generic management lists advertise `Esc:term`.
- [x] T203 Refresh `SPEC-2` artifacts to describe hint parity with the supplemental escape contract.
- [x] T204 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 28: Ctrl+G Management Focus Cycle

- [x] T205 [P] Write RED test: `Ctrl+G, Tab` on a non-Branches management tab skips `BranchDetail` and returns focus to `Terminal`.
- [x] T206 [P] Write RED test: `Ctrl+G, Shift+Tab` on a non-Branches management tab skips `BranchDetail` and returns focus to `TabContent`.
- [x] T207 [P] Write RED test: `Branches` still retains the old three-surface focus cycle and can enter `BranchDetail`.
- [x] T208 Update `app.rs` management focus cycling so non-Branches tabs use a two-state `Terminal <-> TabContent` loop while Branches keeps the three-state loop.
- [x] T209 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 29: Responsive Management Split

- [x] T210 [P] Write RED test: `management_split()` uses `50/50` at standard width and `40/60` at wide width.
- [x] T211 [P] Write RED test: `active_session_content_area()` matches the responsive management split at standard and wide widths.
- [x] T212 Update `app.rs` management split helper so wide terminals (`>=120 cols`) keep `40/60` while narrower terminals fall back to `50/50`.
- [x] T213 Refresh `SPEC-2` artifacts to describe the responsive split contract.
- [x] T214 Verify focused tests, workspace checks, and refresh SPEC-2 artifacts.

## Phase 30: Remove Redundant Management Banner

- [x] T215 [P] Write RED test: management render omits the standalone `gwt | ...` banner row.
- [x] T216 [P] Write RED test: the top management row belongs to pane-title chrome once the banner is removed.
- [x] T217 Update `app.rs` management rendering to remove the banner split and let panes consume the full management area.
- [x] T218 Refresh `SPEC-2` artifacts to describe pane-title-only management chrome.
- [x] T219 Verify focused tests, workspace checks, snapshot refresh, and SPEC-2 artifact sync.

## Phase 31: Restore Terminal-Focused Footer Mnemonics

- [x] T220 [P] Write RED test: terminal-focused footer hints include the global management shortcuts (`Ctrl+G,b/i/s`, `Ctrl+G,g`).
- [x] T221 [P] Write RED test: terminal-focused footer hints include the session/layout/help shortcuts (`Ctrl+G,c`, `Ctrl+G,[]/1-9`, `Ctrl+G,z`, `Ctrl+G,?`).
- [x] T222 Update `app.rs` terminal hint text so Terminal focus advertises the restored global workspace mnemonics.
- [x] T223 Refresh `SPEC-2` artifacts to describe terminal-focused footer-hint parity.
- [x] T224 Verify focused tests, workspace checks, and SPEC-2 artifact sync.

## Phase 32: Make Terminal Footer Hints Survive Standard Width

- [x] T225 [P] Write RED test: the compact grouped terminal footer hint remains visible at `80x24`.
- [x] T226 [P] Write RED test: `C-g Tab:focus` and `^C×2` remain visible in the terminal footer at `80x24`.
- [x] T227 Update `status_bar.rs` and `app.rs` so narrow terminal footers compact context and use grouped shortcut notation.
- [x] T228 Refresh `SPEC-2` artifacts to describe the standard-width compact-footer contract precisely.
- [x] T229 Verify focused tests, snapshot refresh, workspace checks, and SPEC-2 artifact sync.

## Phase 33: Compact Management Footers at Standard Width

- [x] T230 [P] Write RED test: standard-width Branches-list footer keeps its compact hints visible.
- [x] T231 [P] Write RED test: standard-width Branch Detail footer keeps its compact hints visible.
- [x] T232 [P] Write RED test: standard-width generic management footer keeps its compact hints visible.
- [x] T233 Update `app.rs` and `status_bar.rs` so narrow management footers use compact context and hint notation.
- [x] T234 Refresh `SPEC-2` artifacts and verify focused tests, snapshot refresh, and workspace checks.

## Phase 34: Compact Narrow Management Titles

- [x] T235 [P] Write RED test: narrow Branches pane titles collapse to the active tab label instead of the truncated full strip.
- [x] T236 [P] Write RED test: narrow non-Branches management pane titles collapse to the active tab label.
- [x] T237 [P] Write RED test: medium-width non-Branches panes still collapse while extra-wide panes keep the full tab strip.
- [x] T238 Update `app.rs` management title rendering to switch to active-tab-only chrome whenever the full tab strip would truncate.
- [x] T239 Refresh `SPEC-2` artifacts and verify focused tests, snapshot refresh, and workspace checks.

## Phase 35: Compact Narrow Session Titles

- [x] T240 [P] Write RED test: standard-width session pane titles collapse to the active session label instead of the truncated full strip.
- [x] T241 [P] Write RED test: medium-width session panes still collapse while extra-wide panes keep the full session strip.
- [x] T242 [P] Write RED test: extra-wide session panes restore the complete session strip once it fits.
- [x] T243 Update `app.rs` session title rendering to switch to active-session-only chrome whenever the full strip would truncate.
- [x] T244 Refresh `SPEC-2` artifacts and verify focused tests and workspace checks.

## Phase 36: Make Non-Branches Footer Hints Mode-Aware

- [x] T245 [P] Write RED test: Issues detail footer hints show `Esc:back` instead of `Esc:term`.
- [x] T246 [P] Write RED test: Profiles create-mode footer hints show `Esc:cancel` instead of `Esc:term`.
- [x] T247 [P] Write RED test: Settings list keeps `Ctrl+←→:sub-tab` while Git View omits it.
- [x] T248 Update `app.rs` footer hint rendering so non-Branches tabs advertise tab/mode-specific `Esc` and sub-tab affordances.
- [x] T249 Refresh `SPEC-2` artifacts and verify focused tests and workspace checks.

## Phase 37: Consume Branch Detail Esc Before Warn Fallback

- [x] T250 [P] Write RED test: Branch Detail `Esc` with a warn notification returns focus to the list while preserving the warning.
- [x] T251 [P] Write RED test: a second `Esc` from the Branches list still dismisses the warn notification through the normal fallback.
- [x] T252 [P] Write RED test: Branch Detail `Esc` with warn still preserves selected branch/detail/session context.
- [x] T253 Update `app.rs` Branch Detail routing so the local `Esc:back` action is consumed before warn-dismiss fallback runs.
- [x] T254 Refresh `SPEC-2` artifacts and verify focused tests and workspace checks.

## Phase 38: Make Non-Branches Footer Hints Action-Aware

- [x] T255 [P] Write RED test: Git View footer hints advertise `Enter:expand` and `r:refresh` instead of a generic enter action.
- [x] T256 [P] Write RED test: Versions footer hints stay refresh-only and omit a generic enter action.
- [x] T257 [P] Write RED test: Issues list footer hints advertise `Enter:detail`, `/:search`, and `r:refresh`.
- [x] T258 [P] Write RED test: PR Dashboard detail footer hints advertise `Enter:close`, `r:refresh`, and `Esc:back`.
- [x] T259 Update `app.rs` non-Branches management footer hints so each tab advertises only its real primary actions and refresh/search affordances, then refresh `SPEC-2` artifacts and verification evidence.

## Phase 39: Remove Redundant Branch Detail Inner Titles

- [x] T260 [P] Write RED test: `render_detail_content()` keeps Overview body text while omitting the redundant inner `Overview` title.
- [x] T261 [P] Write RED test: `render_detail_content()` keeps session rows while omitting the redundant inner `Sessions` title.
- [x] T262 Update `branches.rs` detail renderers so Overview, SPECs, Git, and Sessions stay borderless/title-free inside the already-labelled pane.
- [x] T263 Refresh branch-detail snapshots to the chrome-light contract.
- [x] T264 Refresh `SPEC-2` artifacts and verify focused tests, snapshots, and workspace checks.

## Phase 40: Preserve Session Count in Compact Session Titles

- [x] T265 [P] Write RED test: standard-width compact session titles keep the active `n/N` position visible alongside the active session label.
- [x] T266 [P] Write RED test: extra-wide session titles keep the full strip and omit the compact `n/N` badge.
- [x] T267 Update `app.rs` compact session title rendering so collapsed titles preserve active index/count context.
- [x] T268 Refresh `SPEC-2` artifacts to describe compact session-title count parity.
- [x] T269 Verify focused tests, workspace checks, and artifact sync.

## Phase 41: Restore Split-Grid Session Title Identity

- [x] T270 [P] Write RED test: split/grid pane titles expose the stable `n:` position for each visible session pane.
- [x] T271 [P] Write RED test: split/grid pane titles keep the session-type icon visible instead of rendering name-only chrome.
- [x] T272 Update `app.rs` grid-session title rendering to include `n:` plus the session-type icon before the session label.
- [x] T273 Refresh grid-layout snapshots and `SPEC-2` artifacts to describe split-grid title parity.
- [x] T274 Verify focused tests, snapshot verification, broad workspace verification, and artifact sync.

## Phase 42: Cache Branch Detail Data Off The Input Path

- [x] T275 [P] Write RED test: `Branches` list navigation switches to cached detail immediately without synchronously reloading the newly selected branch.
- [x] T276 [P] Write RED test: asynchronous branch-detail preload/refresh populates cached detail and updates the selected branch when results arrive.
- [x] T277 Implement branch-detail cache plus asynchronous preload/refresh wiring in `app.rs` / `model.rs` / `branches.rs`.
- [x] T278 Refresh `SPEC-2` artifacts and progress tracking to describe cached asynchronous branch-detail loading.
- [x] T279 Verify focused tests, broad workspace verification, and artifact sync.

## Phase 43: Normalize Reverse Focus Keys And Startup PTY Geometry

- [x] T280 [P] Write RED test: `Shift+Tab` arriving as `KeyCode::Tab` + `KeyModifiers::SHIFT` still moves focus backward on management panes.
- [x] T281 [P] Write RED test: startup terminal-size synchronization seeds the initial shell geometry from the live terminal frame instead of the stale `80x24` default.
- [x] T282 Update `app.rs` focus routing to normalize reverse-tab key encodings before pane cycling decisions.
- [x] T283 Update `main.rs` startup initialization so the model receives the live terminal size before computing the default shell PTY rows/cols.
- [x] T284 Refresh `SPEC-2` artifacts, progress evidence, and verification results for the reverse-focus and startup-geometry fixes.

## Phase 44: Stop Branch List Wrap, Prefer Local Branches, And Keep Nearby Tabs Visible

- [x] T285 [P] Write RED test: Branches list `Up` on the first row and `Down` on the last row stop at the edge instead of wrapping.
- [x] T286 [P] Write RED test: `ViewMode::All` keeps local branches ahead of remote branches for default and name/date sort modes.
- [x] T287 [P] Write RED test: standard-width management pane titles keep the active tab plus nearby tabs visible instead of collapsing to the active label only.
- [x] T288 Update `branches.rs` and `app.rs` to stop Branches-list wraparound, apply local-first ordering in `All`, and render nearby-tab management titles with ellipsis when tabs are hidden.
- [x] T289 Refresh `SPEC-2` artifacts, verification evidence, and progress tracking for the Branches and management-title visibility fixes.

## Phase 45: Default Branches Filter To Local

- [x] T290 [P] Write RED test: Branches default state starts in `ViewMode::Local`.
- [x] T291 [P] Write RED test: the Branches view-mode cycle now starts at `Local` and still reaches `Remote` and `All`.
- [x] T292 [P] Refresh reviewer guidance and snapshots so the initial Branches surface shows `View: Local`.
- [x] T293 Update `branches.rs` so the default `ViewMode` is `Local` without changing the rest of the filter behavior.
- [x] T294 Refresh `SPEC-2` artifacts, verification evidence, and progress tracking for the default-local Branches filter.

## Phase 46: Stabilize Branch Detail Prefetch

- [x] T295 [P] Write RED test: canceling a superseded branch-detail preload worker stops it before it starts loading later branches.
- [x] T296 [P] Write RED test: one branch-detail preload refresh performs exactly one Docker container discovery even when multiple branches are prefetched.
- [x] T297 Update `app.rs` to track and cancel/reap branch-detail preload workers instead of replacing the completion queue and detaching stale workers.
- [x] T298 Update `branches.rs` so branch-detail loading consumes a per-refresh Docker snapshot instead of calling Docker per branch.
- [x] T299 Refresh `SPEC-2` artifacts, verification evidence, and progress tracking for the stabilized Branch Detail preload path.

## Phase 47: Keep Branches List Responsive During Detail Backfill

- [x] T300 [P] Write RED test: one `Tick` does not fully drain a large branch-detail preload queue, so preload work cannot monopolize a frame.
- [x] T301 Update `app.rs` branch-detail event draining to apply a bounded per-tick batch budget while preserving generation/branch validation.
- [x] T302 Refresh `SPEC-2` artifacts and focused verification evidence for the incremental preload-drain contract.

## Phase 48: Keep Terminal Sessions Immediate And Self-Cleaning

- [x] T303 [P] Write RED test: pre-poll PTY draining renders queued PTY output without waiting for a `Tick`.
- [x] T304 [P] Write RED test: `ToggleLayer` immediately resizes PTYs and vt100 parsers to the new visible session content area.
- [x] T305 [P] Write RED test: detected PTY exits automatically remove the dead session tab and clamp the active session index.
- [x] T306 Update the event loop in `main.rs` so PTY output is drained before crossterm polling blocks.
- [x] T307 Update `app.rs` `ToggleLayer` handling to recompute session-pane geometry and resize live PTYs immediately.
- [x] T308 Update `app.rs` PTY exit cleanup to remove exited sessions, preserve a valid active session, and notify with the auto-close result.
- [x] T309 Refresh `SPEC-2` artifacts to describe the new terminal responsiveness and session auto-cleanup contract.
- [x] T310 Run focused workspace-shell verification for event loop, resize sync, and PTY exit cleanup.

## Phase 49: Add Prefixed Focus Escape From Session Panes

- [x] T311 [P] Write RED test: `Ctrl+G,Tab` from a session pane resolves to an explicit forward focus-cycle message instead of being discarded by the prefix state machine.
- [x] T312 [P] Write RED test: `Ctrl+G,Shift+Tab` / `Ctrl+G,BackTab` resolves to the reverse focus-cycle message.
- [x] T313 [P] Write RED test: applying the forward focus-cycle message from `ActiveLayer::Main` reveals management and lands on the next logical pane instead of forwarding `Tab` into the PTY.
- [x] T314 Update the keybind registry and app update path so prefixed focus-cycle commands work consistently from Shell and Agent panes.
- [x] T315 Refresh `SPEC-2` artifacts and focused verification evidence for the prefixed focus-escape contract.

## Phase 50: Restore Hook-Derived Branch Session Discovery

- [x] T316 [P] Write RED test: hook runtime sidecar maps `SessionStart` / `UserPromptSubmit` / `PreToolUse` / `PostToolUse` to `Running` and `Stop` to `WaitingInput`.
- [x] T317 [P] Write RED test: Branches session-summary extraction prefers `Running` over `WaitingInput` when multiple live agent sessions belong to the same branch.
- [x] T318 [P] Write RED test: launched agent PTYs receive stable gwt hook-runtime environment pointing at the correct session runtime record.
- [x] T319 [P] Write RED test: PTY exit and explicit session close persist `Stopped` for agent sessions.
- [x] T320 [P] Write RED test: wide Branches rows render a right-aligned running summary without regressing the left-side branch line.
- [x] T321 [P] Write RED test: waiting rows render a distinct waiting summary.
- [x] T322 [P] Write RED test: narrow Branches rows shorten or omit the live summary before the branch label and core icons disappear.
- [x] T323 Implement hook runtime sidecar persistence, launch env injection, and `Stopped` cleanup updates in `gwt-agent` / `app.rs`.
- [x] T324 Implement branch-scoped live session summary extraction and right-aligned Branches row rendering in `app.rs` / `branches.rs`.
- [x] T325 Refresh `SPEC-2` artifacts and run focused plus broad verification for hook-derived Branches session visibility.

## Phase 51: Normalize Claude Hook Settings Materialization

- [x] T326 [P] Write RED test: `.claude/settings.local.json` regeneration emits Claude's native `hooks` schema and removes the obsolete `managed_hooks` / `user_hooks` keys.
- [x] T327 [P] Write RED test: Claude settings regeneration preserves non-gwt user hooks while replacing stale gwt-managed commands.
- [x] T328 Update `gwt-skills` / `app.rs` launch materialization to generate Claude hook settings through a typed Claude-schema merge instead of the internal hooks-merge schema.
- [x] T329 [P] Write RED test and fix: managed hook assets/settings are written before agent PTY spawn so the first launch turn can emit runtime state.

## Phase 52: PID-Scoped Runtime State And No-Node Hook Commands

- [x] T330 [P] Write RED test: runtime sidecar paths are scoped under the current gwt PID namespace.
- [x] T331 [P] Write RED test: startup/runtime reset clears only the current PID namespace and preserves sibling gwt runtime directories.
- [x] T332 [P] Write RED test: generated Claude and Codex runtime hooks contain no Node-based live-state forward command, include `SessionStart`, and skip `Notification`.
- [x] T333 [P] Write RED test: generated `.codex/hooks.json` preserves user hooks for untracked files and skips tracked `.codex/hooks.json`.
- [x] T334 [P] Write RED test: POSIX runtime hook command writes a runtime sidecar via `GWT_SESSION_RUNTIME_PATH`.
- [x] T335 Implement PID-scoped runtime path/reset helpers in `gwt-agent` and call startup reset from `gwt-tui/src/main.rs`.
- [x] T336 Implement shared Claude/Codex runtime hook generation, `.codex/hooks.json` materialization, and launch-time wiring in `gwt-skills` / `app.rs`.
- [x] T337 Refresh `SPEC-2` / `SPEC-9` artifacts and rerun focused plus broad verification for PID-scoped no-Node hook live-state updates.

## Phase 53: Multi-Agent Branch Spinner Strips

- [x] T338 [P] Write RED test: branch live-session extraction keeps multiple live agent indicators for the same branch instead of collapsing to one summary.
- [x] T339 [P] Write RED test: wide Branches rows render spinner-only indicators without `run ...` / `wait ...` labels.
- [x] T340 [P] Write RED test: branch spinner indicators use the originating agent colors so Claude Code and Codex stay visually distinct.
- [x] T341 Implement multi-indicator branch aggregation and spinner-strip rendering in `app.rs` / `branches.rs`.
- [x] T342 Refresh `SPEC-2` artifacts and rerun focused plus broad verification for multi-agent spinner-strip Branches rendering.

## Phase 54: Branch Spinner Palette Parity

- [x] T343 [P] Write RED test: Claude and Codex branch spinners use `Yellow` and `Cyan` instead of the session-tab palette.
- [x] T344 [P] Write RED test: Gemini branch spinner uses `Magenta` in the Branches strip.
- [x] T345 Implement branch-only built-in agent palette mapping in `app.rs` / `branches.rs` with fallback for custom agents.
- [x] T346 Refresh `SPEC-2` artifacts and rerun focused plus broad verification for the Branches spinner palette mapping.

## Phase 55: Codex Runtime Namespace Sandboxing

- [x] T347 [P] Write RED test: Codex launch adds the `GWT_SESSION_RUNTIME_PATH` parent directory as a writable root.
- [x] T348 Implement Codex runtime namespace writable-root injection in `crates/gwt-agent/src/launch.rs`.
- [x] T349 Refresh `SPEC-2` / `SPEC-9` artifacts and rerun focused plus broad verification for Codex runtime sidecar writes.

## Phase 56: Materialized Codex Runtime Namespace Wiring

- [x] T350 [P] Write RED test: materialized Codex launches append the runtime namespace writable root after the persisted session id is known.
- [x] T351 Implement materialized Codex runtime namespace augmentation in `crates/gwt-tui/src/app.rs`.
- [x] T352 Refresh `SPEC-2` / `SPEC-9` artifacts and rerun focused plus broad verification for the materialized Codex runtime writable-root path.

## Phase 57: Tracked Legacy Codex Hook Migration

- [x] T353 [P] Write RED test: materialized Codex launches migrate tracked legacy `.codex/hooks.json` runtime hooks to the no-Node form before the session starts.
- [x] T354 Implement tracked legacy Codex runtime-hook migration in `crates/gwt-skills/src/settings_local.rs` and launch materialization coverage in `crates/gwt-tui/src/app.rs`.
- [x] T355 Refresh `SPEC-2` / `SPEC-9` artifacts and rerun focused plus broad verification for tracked legacy Codex runtime-hook migration.

## Phase 58: Interactive Codex Launch Bootstrap

- [x] T356 [P] Write RED test: successful materialized Codex launches bootstrap a PID-scoped `Running` runtime sidecar before the first interactive hook event arrives.
- [x] T357 Implement launch-time runtime-state bootstrap in `crates/gwt-tui/src/app.rs` and keep failed launches sidecar-free.

## Phase 59: Terminal Input Trace For IME Investigation

- [x] T358 [P] Write RED test: `Ctrl+G,y` is no longer registered or bound after removing the explicit terminal IME mode workaround.
- [x] T359 [P] Write RED test: opt-in input tracing appends JSONL records for keybind decisions and PTY-forwarded bytes.
- [x] T360 [P] Write RED test: raw `crossterm` key events are serialized into the trace file with stable stage metadata.
- [x] T361 Implement `GWT_INPUT_TRACE_PATH`-gated input tracing and remove terminal IME mode state, footer/help affordances, toggle notification, and key-guard logic from `crates/gwt-tui/src/input_trace.rs`, `crates/gwt-tui/src/event.rs`, `crates/gwt-tui/src/main.rs`, `crates/gwt-tui/src/app.rs`, `crates/gwt-tui/src/input/keybind.rs`, `crates/gwt-tui/src/message.rs`, `crates/gwt-tui/src/model.rs`, and `crates/gwt-tui/src/widgets/status_bar.rs`.
- [x] T362 Refresh `SPEC-2` artifacts and README docs, then rerun focused plus broad verification for the input-trace investigation contract.
