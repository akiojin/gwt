# Workspace Shell — Tasks

## Phase 0: Implementation Details Documentation

- [x] T-W01 Document complete keybinding map (32 bindings) in spec.md
- [x] T-W02 Document Ctrl+G prefix state machine in spec.md
- [x] T-W03 Document session persistence TOML schema in spec.md
- [x] T-W04 Create data-model.md with all state structs
- [x] T-W05 Create research.md with architecture decisions
- [x] T-W06 Create quickstart.md with validation flow

## Phase 1: Help Overlay Auto-Collection

- [ ] T001 [P] Write RED test: help overlay output contains all registered keybindings.
- [ ] T002 [P] Write RED test: help overlay does not contain unregistered key sequences.
- [ ] T003 Define keybinding registry data structure (key sequence, description, category).
- [ ] T004 Populate registry from existing keybind.rs match arms.
- [ ] T005 Render help overlay (Ctrl+G,?) using registry data, grouped by category.
- [ ] T006 Verify help overlay tests pass GREEN.

## Phase 2: Session Persistence Improvement

- [ ] T007 [P] Write RED test: session save/restore round-trip preserves display mode, panel visibility, active tab.
- [ ] T008 [P] Write RED test: corrupted session file triggers graceful fallback with warning.
- [ ] T009 [P] Write RED test: missing session directory is auto-created on save.
- [ ] T010 Extend session persistence TOML schema with display_mode, panel_visible, active_management_tab fields.
- [ ] T011 Implement save logic for new fields.
- [ ] T012 Implement restore logic with graceful fallback for missing/corrupted fields.
- [ ] T013 Verify session persistence tests pass GREEN.

## Phase 3: Git View Tab

- [ ] T014 [P] Write RED test: Git View tab renders recent commit list from git log.
- [ ] T015 [P] Write RED test: Git View tab handles empty repository gracefully.
- [ ] T016 Implement Git View management tab component.
- [ ] T017 Wire Git View into management panel tab navigation.
- [ ] T018 Verify Git View tests pass GREEN.

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

- [x] T030 [P] Write RED test: detail section cycles through Overview/SPECs/GitStatus/Sessions/Actions with Tab
- [x] T031 Implement Overview section: branch name, head, worktree path, category
- [x] T032 Implement SPECs section: placeholder with branch name
- [x] T033 Implement Git Status section: placeholder with branch name
- [x] T034 Implement Sessions section: placeholder "No active sessions"
- [x] T035 Implement Actions section: Launch Agent, Open Shell, Delete Worktree

### 4.4: Actions Implementation

- [x] T036 [P] Write RED test: Agent launch from Actions sets pending flag
- [x] T037 [P] Write RED test: Shell launch sets pending flag
- [x] T038 [P] Write RED test: Worktree delete sets pending flag
- [x] T039 Implement agent launch action: set pending_launch_agent flag
- [x] T040 Implement shell launch action: set pending_open_shell flag
- [x] T041 Implement worktree delete action: set pending_delete_worktree flag

### 4.5: Integration

- [x] T042 Add detail_section, detail_action_selected, pending flags to BranchesState
- [x] T043 Add BranchDetailMessage variants to BranchesMessage
- [x] T044 Route Tab/Shift+Tab to detail section cycling in route_key_to_management
- [x] T045 Route Enter in Actions section to appropriate action handlers
- [x] T046 Write 15 tests for branch detail state transitions and rendering
- [x] T047 Update all E2E snapshot tests

## Phase 5: Regression and Polish

- [ ] T048 Run full existing test suite and verify no regressions.
- [ ] T049 Run `cargo clippy` and `cargo fmt` on all changed files.
- [ ] T050 Verify coverage >= 90%.
- [ ] T051 Run Skill tool with skill: "simplify".
- [ ] T052 Update SPEC-2 metadata phase to Done.
