# Project Workspace — Tasks

## Phase 1: Repo Detection + Initialization Layer

- [ ] T001 [P] Write RED test: detect_repo_type returns NonRepo for empty directory
- [ ] T002 [P] Write RED test: detect_repo_type returns Normal for git repository
- [ ] T003 [P] Write RED test: detect_repo_type returns Bare for bare repository
- [ ] T004 Add RepoType enum (Normal/Bare/NonRepo) and detect_repo_type() to gwt-git
- [ ] T005 Add ActiveLayer::Initialization variant to model.rs
- [ ] T006 Write RED test: Model starts in Initialization when repo_type is NonRepo
- [ ] T007 Update main.rs to detect repo type and set initial ActiveLayer
- [ ] T008 Create screens/initialization.rs with InitializationState and render()
- [ ] T009 Write render test: initialization screen shows URL input prompt
- [ ] T010 Wire Initialization layer into app.rs view()
- [ ] T011 Block Ctrl+G prefix when in Initialization layer
- [ ] T012 Verify Phase 1 tests pass GREEN

## Phase 2: Clone Wizard

- [ ] T013 [P] Write RED test: clone_repo succeeds with valid URL
- [ ] T014 [P] Write RED test: clone_repo fails with invalid URL
- [ ] T015 Add clone_repo(url, target, depth) to gwt-git
- [ ] T016 Add InitializationMessage variants: InputChar, Backspace, StartClone, CloneProgress, CloneSuccess, CloneError
- [ ] T017 Implement update() for InitializationState: URL input, clone trigger, progress, success/error
- [ ] T018 Implement render() for clone progress state (progress bar, status text)
- [ ] T019 On CloneSuccess: call Model::reset(new_repo_root) and transition to Management
- [ ] T020 Add Model::reset(repo_path) method to reload all state
- [ ] T021 Write E2E test: initialization → type URL → clone success → Management layer
- [ ] T022 Verify Phase 2 tests pass GREEN

## Phase 3: Bare Migration + Pre-commit Hook

- [ ] T023 [P] Write RED test: bare repo shows migration error screen
- [ ] T024 Add bare repo migration error rendering to initialization.rs
- [ ] T025 [P] Write RED test: install_develop_protection creates pre-commit hook
- [ ] T026 [P] Write RED test: pre-commit hook preserves existing hooks
- [ ] T027 Implement install_develop_protection(repo_path) in gwt-git
- [ ] T028 Call install_develop_protection after successful clone
- [ ] T029 Verify Phase 3 tests pass GREEN

## Phase 4: Regression and Polish

- [ ] T030 Update E2E snapshots: cargo insta test --accept
- [ ] T031 Run cargo clippy --workspace --all-targets -- -D warnings
- [ ] T032 Verify coverage >= 90%
- [ ] T033 Update SPEC-10 metadata phase to Done
