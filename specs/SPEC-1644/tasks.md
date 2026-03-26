# Tasks

## Phase 0: Ownership boundary refresh

- [x] T001 Refresh `#1644` issue title/body so it acts as the canonical local Git backend owner
- [x] T002 Expand `#1644` artifacts to cover local Git backend ownership, cache/invalidation, and adjacent-spec boundaries
- [x] T003 Add cross-spec boundary notes to `#1654`, `#1647`, `#1714`, `#1643`, and `#1649`

## Phase 1: Domain rewrite

- [x] T101 Rewrite `#1644` artifacts as the canonical ref/worktree domain spec
- [x] T102 Remove Sidebar-specific assumptions from the spec text and checklists

## Phase 2: User Story 1 - Ref inventory projections

- [x] T201 [US1] Define `local / remote / all` projection semantics and canonical entry rules
- [x] T202 [US1] Define same-name local/remote ref handling and gone/upstream projection behavior
- [x] T203 [US1] Add tests for inventory projection and mode switching

## Phase 3: User Story 2 - Resolve refs to worktree actions

- [x] T301 [US2] Define `create worktree` vs `focus existing worktree` resolution rules
- [x] T302 [US2] Define ambiguity behavior when multiple worktree instances map to one ref
- [x] T303 [US2] Add tests for remote-only, local-no-worktree, local-with-worktree, and ambiguous cases

## Phase 4: User Story 3 - Worktree instance meaning and metadata ownership

- [x] T401 [US3] Define canonical worktree-instance projection fields for display name, linkage, tool usage, divergence, and safety
- [x] T402 [US3] Reconfirm cleanup, branch protection, and PR linkage rules as worktree-domain behavior
- [x] T403 [US3] Add regression tests for display-name fallback, linkage, and safety ownership

## Phase 5: User Story 4 - Stable worktree identity for execution sessions

- [x] T501 [US4] Define stable worktree identity and execution-session mapping rules
- [x] T502 [US4] Add tests that session-to-worktree identity survives branch label/display changes

## Phase 6: User Story 5 - Adjacent-spec backend ownership

- [x] T601 [US5] Define local Git backend ownership boundaries versus `#1654`, `#1647`, `#1714`, `#1643`, and `#1649`
- [x] T602 [US5] Add representative owner-mapping examples for cache/invalidation, project orchestration, GitHub integration, and PR lifecycle

## Phase 7: Validation

- [x] T701 Run projection/resolution tests for `local / remote / all`
- [x] T702 Run cleanup / safety / display-name regression tests
- [x] T703 Verify adjacent specs can consume the projection without becoming local Git backend owners
