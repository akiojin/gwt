# Tasks: SPEC-1787 — Workspace Initialization and SPEC-Driven Workflow

## Phase 0: Setup

- [ ] T001: Read and confirm all affected files compile before changes
  - `cargo build -p gwt-core -p gwt-tui`
  - `cargo test -p gwt-core -p gwt-tui`

## Phase 1: Bare Abolition (US2 — FR-006 through FR-011)

- [ ] T002: [P] Write tests asserting `RepoType` enum has no `Bare` variant and `detect_repo_type()` returns `Normal` for standard git repos
  - `crates/gwt-core/src/git/repository.rs`
  - Update existing tests in `main.rs` (lines 43-80) to remove Bare test cases

- [ ] T003: [P] Remove `RepoType::Bare` variant from enum, delete `is_bare_repository()`, delete `find_bare_repo_in_dir()`, update `detect_repo_type()` to skip Bare detection
  - `crates/gwt-core/src/git/repository.rs` (lines 41-52, 112-154, 157-179)

- [ ] T004: [P] Delete `crates/gwt-core/src/config/bare_project.rs`, remove re-export from `crates/gwt-core/src/config/mod.rs` (line 23)

- [ ] T005: [P] Remove Bare detection and `WorktreeLocation::Sibling` from `WorktreeManager::new()`
  - `crates/gwt-core/src/worktree/manager.rs` (lines 43-52)

- [ ] T006: Simplify `resolve_repo_root()` — remove `find_bare_repo_in_dir` fallback, update tests
  - `crates/gwt-tui/src/main.rs` (lines 27-36, 43-80)

- [ ] T007: [P] Remove `CloneType::Bare`, `CloneType::BareShallow`, `CloneStep::TypeSelect` from Clone Wizard. Change clone command to `git clone --depth=1 -b develop <url>` with fallback
  - `crates/gwt-tui/src/screens/clone_wizard.rs` (lines 14-34, 116-129)

- [ ] T008: [P] Delete `crates/gwt-tui/src/screens/migration_dialog.rs`, remove all references in `app.rs` and `model.rs`

- [ ] T009: Fix all remaining compilation errors — grep for `Bare`, `BareProjectConfig`, `MigrationDialog`, `Sibling` across crates/
  - All affected files

- [ ] T010: Verify Phase 1 — `cargo build`, `cargo test`, `cargo clippy`, grep confirms no Bare references (SC-007, SC-008, SC-009)

## Phase 2: Initialization Flow (US1 — FR-001 through FR-005, FR-021)

- [ ] T011: Write test: `Model::new()` with NonRepo/Empty path sets `ActiveLayer::Initialization`
  - `crates/gwt-tui/src/model.rs`

- [ ] T012: Add `ActiveLayer::Initialization` variant to enum
  - `crates/gwt-tui/src/model.rs` (line 29-32)

- [ ] T013: Extract data loading from `run()` into `Model::load_all_data(repo_root: &Path)`
  - `crates/gwt-tui/src/app.rs` (lines 2062-2086)
  - `crates/gwt-tui/src/model.rs`

- [ ] T014: Implement `Model::reset(new_repo_root: PathBuf)` — update repo_root, clear sessions, call load_all_data, set SPECs tab
  - `crates/gwt-tui/src/model.rs`

- [ ] T015: In `Model::new()`, call `detect_repo_type()` and set `ActiveLayer::Initialization` for NonRepo/Empty
  - `crates/gwt-tui/src/model.rs` (lines 221-256)

- [ ] T016: Add `ActiveLayer::Initialization` draw branch in app.rs — render fullscreen Clone Wizard
  - `crates/gwt-tui/src/app.rs`

- [ ] T017: Add `ActiveLayer::Initialization` event handling — block Ctrl+G, Esc exits TUI
  - `crates/gwt-tui/src/app.rs`

- [ ] T018: Add `render_fullscreen()` to Clone Wizard — centered URL input without overlay border
  - `crates/gwt-tui/src/screens/clone_wizard.rs`

- [ ] T019: On clone complete in Initialization mode, call `Model::reset(cloned_path)` to transition to SPECs tab
  - `crates/gwt-tui/src/app.rs`

- [ ] T020: Verify Phase 2 — launch from empty dir shows init screen, clone transitions to SPECs tab (SC-001, SC-003)

## Phase 3: develop Commit Protection (US5 — FR-018 through FR-020)

- [ ] T021: [P] Write test: pre-commit hook script blocks commit on develop, allows on feature branch
  - `crates/gwt-core/src/git/hooks.rs` (new)

- [ ] T022: [P] Create `crates/gwt-core/src/git/hooks.rs` with `install_pre_commit_hook(repo_root)` — generates hook script, merges with existing hooks
  - Hook uses `# gwt-develop-guard-start` / `# gwt-develop-guard-end` markers

- [ ] T023: Register `hooks` module in `crates/gwt-core/src/git/mod.rs`

- [ ] T024: Call `install_pre_commit_hook()` after clone completion and on first launch in existing repos
  - `crates/gwt-tui/src/app.rs`

- [ ] T025: [P] Add develop direct-commit prohibition rule to `AGENTS.md`

- [ ] T026: Verify Phase 3 — `git commit` on develop blocked, allowed on feature (SC-006)

## Phase 4: SPEC/Issue Launch Actions (US3 — FR-012 through FR-015)

- [ ] T027: Add guide message to SPECs tab when `specs/` is empty: "No SPECs yet. Press [n] to create one."
  - `crates/gwt-tui/src/screens/specs.rs` (render function, line 247)

- [ ] T028: Enhance `LaunchAgent` in SPECs tab to derive `feature/feature-{N}` branch name from SPEC ID
  - `crates/gwt-tui/src/screens/specs.rs` (lines 76-93, 127, 145)

- [ ] T029: [P] Add `LaunchAgent` message and key binding to Issues tab, derive `feature/issue-{N}` branch name
  - `crates/gwt-tui/src/screens/issues.rs`

- [ ] T030: Extend `WizardState::open_for_spec()` to set `is_new_branch = true` and pre-fill `feature/feature-{N}`
  - `crates/gwt-tui/src/screens/wizard.rs` (lines 428-450)

- [ ] T031: Add `WizardState::open_for_issue()` with similar branch derivation logic
  - `crates/gwt-tui/src/screens/wizard.rs`

- [ ] T032: Verify Phase 4 — SPEC Launch Agent opens wizard with correct branch name (SC-004 partial)

## Phase 5: SPEC Drafting Skill + Workflow (US4 — FR-016, FR-017)

- [ ] T033: Create `.claude/skills/gwt-spec-draft/SKILL.md` — integrate brainstorming, gwt-spec-register/clarify/plan/tasks, multiple SPEC branch creation
  - `.claude/skills/gwt-spec-draft/SKILL.md` (new)

- [ ] T034: Add "New SPEC" key binding (`n`) to SPECs tab, trigger SPEC drafting agent launch on develop
  - `crates/gwt-tui/src/screens/specs.rs`

- [ ] T035: Add `WizardState::open_for_spec_drafting()` — sets CWD to repo root (develop), injects SPEC drafting skill context
  - `crates/gwt-tui/src/screens/wizard.rs`

- [ ] T036: Verify Phase 5 — press `n` in SPECs tab, agent launches on develop with skill context (SC-004, SC-005)

## Phase 6: Polish / Cross-Cutting

- [ ] T037: Full verification pass: `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt`

- [ ] T038: Update SPEC-1647 metadata to status: "closed" (superseded by SPEC-1787)
  - `specs/SPEC-1647/metadata.json`

- [ ] T039: Commit all changes and push
