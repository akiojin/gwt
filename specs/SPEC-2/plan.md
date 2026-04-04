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

### Phase 9: Branch Mnemonic Restoration (5 tasks)
Restore the old-TUI branch-local mnemonic shortcuts that make Branches usable as a daily
entry point without requiring Ctrl+G for every follow-up action.

9.1: Branch-local shortcuts (3 tasks)
- Restore `m` as the Branches-local view-mode cycle.
- Restore `v` as a direct jump from Branches to Git View.
- Restore `f` as a search alias and `?` / `h` as local help entry points.

9.2: UX polish and regression (2 tasks)
- Update branch-specific footer hints to advertise the restored mnemonic set.
- Add focused regression coverage and re-run workspace verification.

### Phase 10: Branch Detail Sessions Restoration (5 tasks)
Restore the Branch Detail `Sessions` pane from a count-only placeholder to a branch-scoped
session summary list so the Branches view can function as the primary workspace entry point.

10.1: Session summary extraction (2 tasks)
- Build a lightweight render-time session summary from the selected branch without adding
  new persistent state.
- Limit the scope to `app.rs` and `branches.rs` to avoid reopening unrelated dirty files.

10.2: Sessions pane rendering (2 tasks)
- Replace the count-only placeholder with a typed list that shows Shell/Agent, session name,
  and an active-session marker for the current tab.
- Preserve the existing empty-state fallback when no sessions match the selected branch.

10.3: Regression and verification (1 task)
- Add focused extraction/render coverage and re-run workspace verification.

### Phase 11: Branch Detail Session Focus Actions (5 tasks)
Turn the restored `Sessions` pane into an actionable branch-first surface so the user can move
from the selected branch directly into one of its running sessions without leaving Branches first.

11.1: Session row selection (2 tasks)
- Track a lightweight selection index for branch-scoped session rows inside `BranchesState`.
- Keep the selection clamped/reset when the branch list changes or when session rows disappear.

11.2: Focus handoff (2 tasks)
- Route `Up/Down` inside the `Sessions` section to row selection instead of Docker controls.
- Route `Enter` inside the `Sessions` section to activate the selected session and move focus to the terminal pane.

11.3: Regression and verification (1 task)
- Add focused routing/render coverage and re-run workspace verification.

### Phase 12: Status Bar Restoration (5 tasks)
Restore the old-TUI footer model so the bottom line carries workspace context again instead of
acting as a keybind-hints-only strip.

12.1: Footer context contract (2 tasks)
- Render current session summary, current branch context, and session type / agent type in the bottom status bar.
- Preserve context-sensitive keybind hints and notification visibility within the same single-line footer surface.

12.2: Wiring and regression coverage (2 tasks)
- Route the main view footer through the shared status-bar widget again instead of the bespoke hints-only renderer.
- Add focused render coverage for shell sessions, agent sessions, and Branches focus hints so the footer contract stays stable.

12.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

### Phase 13: Branch Detail Direct Actions (5 tasks)
Restore the old-TUI direct-action ergonomics inside Branch Detail so the selected branch remains
actionable even after focus leaves the top list.

13.1: Direct branch actions (2 tasks)
- Route `Shift+Enter` in Branch Detail to open a shell for the selected branch when the active
  section is not `Sessions`.
- Route `Ctrl+C` in Branch Detail to open the delete-worktree confirmation when the active
  section is not `Sessions`.

13.2: Section-sensitive hints (2 tasks)
- Replace the generic Branch Detail footer hint with section-aware hints that explain when
  `Enter` focuses a session versus when direct branch actions are available.
- Keep Docker lifecycle hints visible in the Overview section without touching shared layout code.

13.3: Verification (1 task)
- Re-run focused and broad workspace verification and refresh SPEC-2 artifacts.

## Dependencies

- SPEC-3 (Agent Management): Agent detection for agent launch action
- SPEC-4 (GitHub): Git status and PR data for detail sections
- SPEC-10 (Workspace): Worktree management for delete action

## Verification

1. `cargo test -p gwt-tui` — all pass
2. `cargo test -p gwt-tui --test snapshot_e2e` — all E2E pass
3. `cargo clippy -p gwt-tui --all-targets -- -D warnings` — clean
4. Manual: launch gwt-tui, navigate branches, verify detail panel updates on cursor move
