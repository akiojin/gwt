# Implementation Plan: SPEC-2 — Workspace Shell

## Summary

Complete the workspace shell with branch detail view, help overlay, session persistence, and SPECs tab removal. The branch detail view replaces the independent SPECs management tab by integrating SPEC display into a split-layout branch detail panel.

## Technical Context

- **Architecture**: Elm Architecture (Model/Message/Update/View) in gwt-tui
- **Keybind system**: Ctrl+G prefix state machine in `input/keybind.rs`
- **Screens**: `screens/branches.rs` (primary target for branch detail)
- **List rendering**: Uses `ListState` + `render_stateful_widget` for scrollable lists
- **Shared utilities**: `screens/mod.rs` — clamp_index, move_up/down, list_item_style, centered_rect

## Constitution Check

- Spec before implementation: yes, this SPEC documents all requirements
- Test-first: all phases start with RED tests
- No workaround-first: branch detail is a proper implementation, not a hack
- Minimal complexity: each phase is independent and separately verifiable

## Complexity Tracking

| Risk | Mitigation |
|------|-----------|
| Branch detail sections need data from multiple sources | Load on cursor move, cache per branch |
| SPECs tab removal affects tab indexing and keybinds | Update all ManagementTab references |
| Agent launch from detail needs simplified wizard | Reuse WizardState with branch pre-filled |
| Worktree delete is destructive | Confirmation dialog required |

## Phased Implementation

### Phase 1: Help Overlay Auto-Collection (6 tasks)
Implement keybinding registry auto-collection for Ctrl+G,? help overlay.

### Phase 2: Session Persistence Improvement (7 tasks)
Extend save/restore to include display_mode, management panel state.

### Phase 3: Git View Tab (5 tasks)
Implement Git View management tab component.

### Phase 4: Branch Detail View (26 tasks)
4.1: Remove SPECs tab (4 tasks)
4.2: Branch detail split layout (4 tasks)
4.3: Detail sections — Overview/SPECs/GitStatus/Sessions/Actions (6 tasks)
4.4: Actions — agent launch, shell, worktree delete (6 tasks)
4.5: Integration and testing (6 tasks)

### Phase 5: Regression and Polish (5 tasks)

### Phase 8: Branch-First UX Restoration (5 tasks)
Reconcile remaining old-TUI branch-first UX requirements that are already present in `spec.md`
but not fully reflected in the new TUI implementation.

8.1: Branch list display (2 tasks)
- Remove category headers and locality badges from the branch list.
- Render `name + worktree indicator + HEAD indicator` in a stable old-TUI style.

8.2: Primary branch actions (2 tasks)
- Restore `Enter=Wizard`, `Shift+Enter=Shell`, `Space=select detail`, `Ctrl+C=delete worktree`
  on the Branches tab without regressing existing focus-aware routing.
- Update contextual footer hints so Branches communicates the restored actions directly.

8.3: Regression and verification (1 task)
- Add focused routing/render coverage and re-run workspace verification.

## Dependencies

- SPEC-3 (Agent Management): Agent detection for agent launch action
- SPEC-4 (GitHub): Git status and PR data for detail sections
- SPEC-10 (Workspace): Worktree management for delete action

## Verification

1. `cargo test -p gwt-tui` — all pass
2. `cargo test -p gwt-tui --test snapshot_e2e` — all E2E pass
3. `cargo clippy -p gwt-tui --all-targets -- -D warnings` — clean
4. Manual: launch gwt-tui, navigate branches, verify detail panel updates on cursor move
