# Workspace Shell -- Tasks

## Phase 0: Implementation Details Documentation

- [x] T-W01 Document complete keybinding map (18 bindings) in spec.md
- [x] T-W02 Document Ctrl+G prefix state machine in spec.md
- [x] T-W03 Document session persistence TOML schema in spec.md

## Phase 1: Help Overlay Auto-Collection

- [ ] T001 [P] Write RED test: help overlay output contains all registered keybindings (Ctrl+G,c / Ctrl+G,x / Ctrl+G,z / etc.).
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
- [ ] T017 Wire Git View into management panel tab navigation at index 5.
- [ ] T018 Verify Git View tests pass GREEN.

## Phase 4: Branch Detail View + SPECs Tab Removal

- [ ] T022 Remove ManagementTab::Specs variant from model.rs (7 tabs instead of 8)
- [ ] T023 Remove specs tab routing from app.rs (route_key_to_management, render_management_tab_content)
- [ ] T024 Create screens/branch_detail.rs with BranchDetailState:
  - sections: Overview, SPECs, GitStatus, Sessions, Actions
  - active_section: usize (Tab cycles)
  - spec_items, files, commits, sessions, pr_status
- [ ] T025 Add BranchDetailMessage: NextSection, PrevSection, MoveUp, MoveDown, Select, Refresh, Back
- [ ] T026 Implement Overview section: branch name, head, worktree path, linked Issues, PR
- [ ] T027 Implement SPECs section: read specs/ from worktree path, list SPEC metadata
- [ ] T028 Implement GitStatus section: staged/unstaged/untracked files, recent commits
- [ ] T029 Implement Sessions section: active sessions on this branch
- [ ] T030 Implement Actions section: Launch Agent (agent select only), Open Shell (worktree cwd), Delete Worktree (with confirmation)
- [ ] T031 Wire into branches.rs: Enter on branch → BranchDetail, Esc → back to list
- [ ] T032 Add BranchDetailState to Model, route messages in app.rs
- [ ] T033 Write 10+ tests for branch detail state transitions and rendering
- [ ] T034 Update E2E snapshot tests

## Phase 5: Regression and Polish

- [ ] T019 Run full existing test suite and verify no regressions.
- [ ] T020 Run `cargo clippy` and `cargo fmt` on all changed files.
- [ ] T021 Update SPEC-2 progress artifacts with verification results.
