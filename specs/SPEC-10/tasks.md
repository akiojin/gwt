# Project Workspace — Tasks

## Phase 1: Repo Detection + Initialization Layer

- [x] T001 [P] Write RED test: detect_repo_type returns NonRepo for empty directory
- [x] T002 [P] Write RED test: detect_repo_type returns Normal for git repository
- [x] T003 [P] Write RED test: detect_repo_type returns Bare for bare repository
- [x] T004 Add RepoType enum (Normal/Bare/NonRepo) and detect_repo_type() to gwt-git
- [x] T005 Add ActiveLayer::Initialization variant to model.rs
- [x] T006 Write RED test: Model starts in Initialization when repo_type is NonRepo
- [x] T007 Update main.rs to detect repo type and set initial ActiveLayer
- [x] T008 Create screens/initialization.rs with InitializationState and render()
- [x] T009 Write render test: initialization screen shows URL input prompt
- [x] T010 Wire Initialization layer into app.rs view()
- [x] T011 Block Ctrl+G prefix when in Initialization layer
- [x] T012 Verify Phase 1 tests pass GREEN

## Phase 2: Clone Wizard

- [x] T013 [P] Write RED test: clone_repo succeeds with valid URL
- [x] T014 [P] Write RED test: clone_repo fails with invalid URL
- [x] T015 Add clone_repo(url, target, depth) to gwt-git
- [x] T016 Add InitializationMessage variants: InputChar, Backspace, StartClone, CloneProgress, CloneSuccess, CloneError
- [x] T017 Implement update() for InitializationState: URL input, clone trigger, progress, success/error
- [x] T018 Implement render() for clone progress state (progress bar, status text)
- [x] T019 On CloneSuccess: call Model::reset(new_repo_root) and transition to Management
- [x] T020 Add Model::reset(repo_path) method to reload all state
- [x] T021 Write E2E test: initialization → type URL → clone success → Management layer
- [x] T022 Verify Phase 2 tests pass GREEN

## Phase 3: Bare Migration + Pre-commit Hook

- [x] T023 [P] Write RED test: bare repo shows migration error screen
- [x] T024 Add bare repo migration error rendering to initialization.rs
- [x] T025 [P] Write RED test: install_develop_protection creates pre-commit hook
- [x] T026 [P] Write RED test: pre-commit hook preserves existing hooks
- [x] T027 Implement install_develop_protection(repo_path) in gwt-git
- [x] T028 Call install_develop_protection after successful clone
- [x] T029 Verify Phase 3 tests pass GREEN

## Phase 4: Regression and Polish

- [x] T030 Update E2E snapshots: cargo insta test --accept
- [x] T031 Run cargo clippy --workspace --all-targets -- -D warnings
- [x] T032 Verify coverage >= 90%
- [x] T033 Update SPEC-10 metadata phase to Done

## Phase 5: Project Index Runtime Bootstrap

- [x] T034 Add RED tests for `gwt-core` runtime bootstrap asset writing, idempotency, and broken-venv rebuild behavior.
- [x] T035 Implement repo-tracked `chroma_index_runner.py` + requirements asset bootstrap in `gwt-core`.
- [x] T036 Wire runtime bootstrap into `gwt-git::initialize_workspace()` with injected-home tests.
- [x] T037 Wire startup/clone warning degradation into `gwt-tui` and verify focused helper tests.

## Phase 6: Python Runtime Hardening

- [x] T038 Add RED tests for validated bootstrap Python candidate selection, including ignored Windows launcher stubs.
- [x] T039 Add RED tests for user-facing Python install guidance in startup / clone warning notifications.
- [x] T040 Implement project-index bootstrap Python validation and guidance-aware error shaping in `gwt-core`.
- [x] T041 Update TUI warning messaging and README docs for managed project-index Python setup, then rerun focused plus broad verification.
