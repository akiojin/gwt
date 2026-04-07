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

## Phase 7: Review Follow-up

- [x] T042 Add RED tests for working Windows Store / launcher Python acceptance, `py -3` propagation, and 3.8→3.9 fallback.
- [x] T043 Add RED tests for stable project-index runtime error classification across startup and clone-completion paths.
- [x] T044 Implement runtime candidate discovery / error classification fixes in `gwt-core`.
- [x] T045 Implement shared notification classification + README corrections in `gwt-tui` / docs.
- [x] T046 Refresh SPEC artifacts and rerun focused plus broad verification.

## Phase 8: Index Lifecycle Redesign (FR-017〜FR-029)

> Bugfix branch: `bugfix/not-work-index`. Single PR `→ develop`.

### Phase 8a: Failing tests (TDD RED)

- [ ] T-IDX-001 [P] Write RED Python test: `crates/gwt-core/runtime/tests/test_e5_prefix.py` covering `passage:` / `query:` prefix application and double-application avoidance
- [ ] T-IDX-002 [P] Write RED Python test: `crates/gwt-core/runtime/tests/test_auto_build_fallback.py` covering missing-index auto-build, `--no-auto-build`, and stderr NDJSON progress
- [ ] T-IDX-003 [P] Write RED Python test: `crates/gwt-core/runtime/tests/test_manifest_diff.py` covering full→incremental, deleted files, mtime/size change detection
- [ ] T-IDX-004 [P] Write RED Python test: `crates/gwt-core/runtime/tests/test_flock.py` covering writer serialization, reader-after-writer, lock release on exception
- [ ] T-IDX-005 [P] Write RED Python test: `crates/gwt-core/runtime/tests/test_issue_ttl.py` covering `last_full_refresh` updates, `status` action TTL output, `--respect-ttl` skip
- [ ] T-IDX-006 [P] Write RED Python test: `crates/gwt-core/runtime/tests/test_repo_layout.py` covering `--repo-hash`/`--worktree-hash`/`--scope` to DB path resolution
- [ ] T-IDX-007 [P] Write RED Rust test: `crates/gwt-core/tests/repo_hash.rs` covering HTTPS/SSH normalization, case insensitivity, hash format
- [ ] T-IDX-008 [P] Write RED Rust test: `crates/gwt-core/tests/worktree_hash.rs` covering symlink canonicalization and relative-path rejection
- [ ] T-IDX-009 [P] Write RED Rust test: `crates/gwt-core/tests/index_paths.rs` covering Issue/SPEC/Files DB path construction
- [ ] T-IDX-010 [P] Write RED Rust test: `crates/gwt-core/tests/watcher_debounce.rs` covering burst debounce, batch size split, gitignore filtering, shutdown
- [ ] T-IDX-011 [P] Write RED Rust test: `crates/gwt-core/tests/worktree_gc.rs` covering orphan removal, existing-worktree preservation, legacy `.gwt/index` removal
- [ ] T-IDX-012 [P] Write RED Rust test: `crates/gwt-core/tests/issue_refresh.rs` covering TTL-expired kick, TTL-skipped no-op, background non-blocking
- [ ] T-IDX-013 Write RED Rust e2e test: `crates/gwt-core/tests/index_runner_spawn.rs` (`#[ignore]`) spawning real Python runner with real e5 model
- [ ] T-IDX-014 Verify all Phase 8a tests are RED via `pytest -q` and `cargo test`

### Phase 8b: Runner redesign (Python)

- [ ] T-IDX-015 Update `crates/gwt-core/runtime/project_index_requirements.txt` to add `sentence-transformers`, `portalocker` [Test: T-IDX-001..006]
- [ ] T-IDX-016 Rewrite `crates/gwt-core/runtime/chroma_index_runner.py` argparse: add `--repo-hash`, `--worktree-hash`, `--scope`, `--no-auto-build`, `--respect-ttl`, `--mode {full|incremental}` [Test: T-IDX-006]
- [ ] T-IDX-017 Implement `resolve_db_path()` helper in runner (computes `~/.gwt/index/<repo>/...`) [Test: T-IDX-006]
- [ ] T-IDX-018 Implement `E5EmbeddingFunction` class with prefix handling, inject into all `get_or_create_collection()` calls [Test: T-IDX-001]
- [ ] T-IDX-019 Implement `acquire_lock(db_path, exclusive: bool)` context manager using `portalocker` [Test: T-IDX-004]
- [ ] T-IDX-020 Implement search-* auto-build fallback: detect missing index, run index-* in-process, emit NDJSON progress on stderr [Test: T-IDX-002]
- [ ] T-IDX-021 Implement manifest read/write helpers and incremental indexing for `index-files` and `index-specs` [Test: T-IDX-003]
- [ ] T-IDX-022 Implement Issue TTL: write `last_full_refresh` in `meta.json`, expose via `status` action, honor `--respect-ttl` [Test: T-IDX-005]
- [ ] T-IDX-023 Drive Phase 8a Python tests to GREEN; remove obsolete code paths [Test: T-IDX-001..006]

### Phase 8c: Rust helpers

- [ ] T-IDX-024 [P] Implement `crates/gwt-core/src/repo_hash.rs` with `normalize_origin_url()` + `compute_repo_hash()` [Test: T-IDX-007]
- [ ] T-IDX-025 [P] Implement `crates/gwt-core/src/worktree_hash.rs` with `compute_worktree_hash()` [Test: T-IDX-008]
- [ ] T-IDX-026 Add `crates/gwt-core/src/index/mod.rs` and `crates/gwt-core/src/index/paths.rs` exposing `gwt_index_db_path(repo, wt, Scope)` and `Scope` enum [Test: T-IDX-009]
- [ ] T-IDX-027 Implement `crates/gwt-core/src/index/manifest.rs` for `manifest.json` read/write (also used by Rust integrity check)
- [ ] T-IDX-028 Implement `crates/gwt-core/src/index/runtime.rs`: `IssueIndexJob`, `WorktreeIndexJob`, `WorktreeGcJob` with tokio task spawning helpers
- [ ] T-IDX-029 Add `sha2`, `notify`, `notify-debouncer-mini`, `fs2` to `crates/gwt-core/Cargo.toml`
- [ ] T-IDX-030 Drive Phase 8c Rust tests to GREEN [Test: T-IDX-007..009, T-IDX-011, T-IDX-012]

### Phase 8d: Watcher

- [ ] T-IDX-031 Implement `crates/gwt-core/src/index/watcher.rs` with `start_watcher(repo_hash, wt_hash, worktree_path) -> WatcherHandle` using `notify-debouncer-mini` (2s debounce)
- [ ] T-IDX-032 Implement 100-file batch size limit and `.gitignore` filtering inside the watcher event loop
- [ ] T-IDX-033 Implement `WatcherHandle::shutdown()` with proper resource release
- [ ] T-IDX-034 Wire watcher batches to spawn `runner index-* --mode incremental` jobs
- [ ] T-IDX-035 Drive `watcher_debounce.rs` to GREEN [Test: T-IDX-010]

### Phase 8e: TUI integration

- [ ] T-IDX-036 Modify `crates/gwt-tui/src/main.rs` startup sequence around `load_initial_data` to spawn tokio tasks: `reconcile_repo`, `refresh_issues_if_stale(15min)`, per-worktree `start_watcher`
- [ ] T-IDX-037 Modify `crates/gwt-tui/src/app.rs::spawn_pty_for_session` (≈line 2875) to ensure watcher exists for the spawned worktree and to kick an integrity check
- [ ] T-IDX-038 Add `remove_worktree_index(repo_hash, wt_hash)` invocation to the Worktree remove handler in `crates/gwt-tui/src/app.rs`
- [ ] T-IDX-039 Manual verification per quickstart additions

### Phase 8f: Skill / launch.rs / docs

- [ ] T-IDX-040 [P] Update `.claude/skills/gwt-search/SKILL.md` with `--repo-hash` / `--worktree-hash` / `--scope` and removal of "Index not found" guidance
- [ ] T-IDX-041 [P] Update `.claude/skills/gwt-spec-search/SKILL.md` likewise
- [ ] T-IDX-042 [P] Update `.claude/skills/gwt-issue-search/SKILL.md` likewise
- [ ] T-IDX-043 [P] Update `.claude/skills/gwt-project-search/SKILL.md` likewise
- [ ] T-IDX-044 Modify `crates/gwt-agent/src/launch.rs` to export `GWT_REPO_HASH` and `GWT_WORKTREE_HASH` to spawned panes

### Phase 8g: Verification & PR

- [ ] T-IDX-045 Run `cargo test -p gwt-core -p gwt-tui` and confirm all GREEN
- [ ] T-IDX-046 Run `pytest -q crates/gwt-core/runtime/tests` and confirm all GREEN
- [ ] T-IDX-047 Run `cargo clippy --all-targets --all-features -- -D warnings` and resolve any warnings
- [ ] T-IDX-048 Run `cargo fmt` and commit any formatting changes
- [ ] T-IDX-049 Manual e2e (5 scenarios from `specs/SPEC-10/quickstart.md` Phase 8 additions)
- [ ] T-IDX-050 Open `bugfix/not-work-index → develop` PR with structured commit history per phase
