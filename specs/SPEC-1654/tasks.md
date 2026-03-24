# Tasks

## Phase 1: Spec topology rewrite

- [x] T001 Rewrite `#1654` artifacts as the canonical workspace shell + execution session spec
- [x] T002 Align `#1644` as the canonical ref/worktree domain spec consumed by the shell
- [x] T003 Retire `#1687` from the active `gwt-spec` set

## Phase 2: User Story 1 - Replace the primary shell surface

- [x] T101 [US1] Replace Sidebar + split-tab assumptions with flat top-level tabs in `App.svelte`
- [x] T102 [US1] Add `agentCanvas` and `branchBrowser` shell state and active-tab routing
- [x] T103 [US1] Preserve non-canvas tabs (`settings`, `issues`, `prs`, `versionHistory`, `projectIndex`, `issueSpec`) as flat top-level tabs
- [x] T104 [US1] Remove remaining split-group presentation remnants from the canonical shell rendering path
- [x] T105 [US1] Make `Agent Canvas` and `Branch Browser` full-window single-surface tabs without persistent side-by-side detail panes

## Phase 3: User Story 2 - Branch Browser materializes worktrees from refs

- [x] T201 [US2] Add a dedicated Branch Browser tab that consumes `#1644` local/remote/all projections
- [x] T202 [US2] Support create/focus worktree actions from selected refs
- [x] T203 [US2] Add Branch Browser tests for mode switching, selection, and action dispatch
- [x] T204 [US2] Render selected branch detail in a full-window layout without left/right split panes

## Phase 4: User Story 3 - Agent Canvas owns execution relationships

- [x] T301 [US3] Add `AgentCanvasState`, `AgentCanvasTile`, `AgentCanvasEdge`, and viewport helpers
- [x] T302 [US3] Render `assistant`, `worktree`, `agent`, and `terminal` tiles on a true canvas surface rather than a fixed grid surrogate
- [x] T303 [US3] Connect worktree-parent edges to launched agent/terminal tiles and keep them stable across drag/pan/zoom viewport changes
- [x] T304 [US3] Add worktree detail popup and visible-tile-only live terminal mounting in the final canvas interaction model
- [x] T305 [US3] Add free placement, tile drag, pan, and zoom interactions that satisfy FR-009
- [x] T306 [US3] Remove the persistent right-side detail pane from `Agent Canvas` and replace it with popup/overlay-only detail surfaces
- [x] T307 [US3] Render agent/terminal session tiles as live terminal surfaces instead of summary surrogates
- [ ] T308 [US3] Improve initial tile composition so major tiles are not clipped or sparsely scattered on first paint
- [ ] T309 [US3] Agent Canvas の「card」用語を「tile」にリネーム（型定義・コンポーネント・CSS・data-testid・永続化キー）。影響: フロントエンド9ファイル/約151箇所 + E2E 1ファイル/5箇所。Rust バックエンドは影響なし。永続化の cardLayouts → tileLayouts マイグレーション含む

## Phase 5: User Story 4 - Window-local persistence and restore

- [x] T401 [US4] Migrate old agent/terminal tab state into canvas tiles
- [x] T402 [US4] Remove split group persistence from the canonical shell path rather than only bypassing it
- [x] T403 [US4] Restore shell/canvas state per window without cross-window contamination after the canvas interaction model is finalized
- [x] T404 [US4] Reconcile persisted shell state with the full-window single-surface model

## Phase 6: Validation

- [x] T601 Run targeted shell/component tests for flat tabs + canvas/browser state against the corrected shell model
- [x] T602 Run e2e coverage for Branch Browser -> worktree -> agent/terminal tile flows including tile interaction
- [x] T603 Re-run multi-window restore verification with the new shell model
- [x] T604 Verify ``Cmd+\`` / ``Ctrl+\`` still performs window cycling when canvas or terminal focus is active after remediation
- [x] T605 Add regression coverage that all top-level tabs occupy the full window without persistent side-by-side detail panes

## Phase 7: Performance and responsiveness

- [x] T701 Remove eager startup warmups from `open_project` so project open stays interactive
- [x] T702 Execute `list_branch_inventory` on native background threads and add slow-path diagnostics
- [x] T703 Remove dead Sidebar code and obsolete shell-only sidebar config from the canonical GUI path
- [x] T704 Add responsive-performance E2E budgets for startup interactive <= 1000ms and maximize/restore interactive <= 300ms
- [x] T705 Add backend benchmark coverage for worktree listing fast paths and keep inventory hot paths observable

## Phase 8: E2E inventory and full-suite reconciliation

- [x] T801 Inventory all current GUI surfaces and all existing Playwright suites for current-shell vs stale coverage status
- [x] T802 Migrate headed UX-critical suites (`agent-canvas-browser`, `agent-terminal`, `branch-worktree`, `dialogs-common`, `project-management`, `responsive-performance`) to the canonical shell
- [x] T803 Add screenshot-backed UX assertions to headed local E2E and wire them into commit-time hooks
- [ ] T804 Migrate remaining stale suites (`open-project-smoke`, `issue-management`, `pr-management`, `git-view`, `cleanup-migration`, `project-mode`, `status-bar`, `settings-config`, `windows-shell-selection`, `issue-cache-sync`, `pr-unknown-retry`, `voice-input-settings`, `tab-layout`, `tab-switch-performance`) to the canonical shell or explicitly retire them
- [ ] T805 Introduce trustworthy E2E coverage measurement and report the percentage only after the full-suite inventory is reconciled
