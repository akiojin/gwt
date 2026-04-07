# Project Workspace — Implementation Plan

## Summary

Add workspace initialization flow to gwt-tui: repo detection on startup, clone wizard for non-repo directories, bare repo migration guidance, develop branch commit protection, and project-index runtime self-repair.

## Technical Context

- **Model**: `crates/gwt-tui/src/model.rs` — add `ActiveLayer::Initialization`
- **App**: `crates/gwt-tui/src/app.rs` — add repo detection in startup, Initialization layer rendering
- **Main**: `crates/gwt-tui/src/main.rs` — detect repo before creating Model
- **New screen**: `crates/gwt-tui/src/screens/initialization.rs` — clone wizard UI
- **gwt-git**: `crates/gwt-git/src/repository.rs` — add repo detection helpers
- **gwt-core**: runtime asset bootstrap for `~/.gwt/runtime/chroma_index_runner.py` and `~/.gwt/runtime/chroma-venv`

## Phased Implementation

### Phase 1: Repo Detection + ActiveLayer::Initialization

1. Add `ActiveLayer::Initialization` to model.rs
2. Add `detect_repo_type(path) -> RepoType` to gwt-git (Normal/Bare/NonRepo)
3. In main.rs: detect repo type before Model::new(), set initial layer accordingly
4. Create `screens/initialization.rs` with URL input + clone progress

### Phase 2: Clone Wizard

1. Implement `clone_repo(url, target_dir)` in gwt-git using `git clone --depth=1`
2. Wire clone into initialization screen: Enter → start clone → progress → success/error
3. On success: Model::reset(new_repo_root) → transition to Management layer

### Phase 3: Bare Repo Migration + Pre-commit Hook

1. Bare repo detection → show migration error screen
2. Pre-commit hook installation after clone
3. Hook preserves existing hooks

### Phase 4: Project Index Runtime Bootstrap

1. Add repo-tracked project-index runtime assets to `gwt-core`
2. Repair runner + managed venv during workspace initialization
3. Repair runner + managed venv during normal startup and surface warning-only degradation

### Phase 5: Index Lifecycle Redesign (FR-017〜FR-029)

Driven by the gwt-search "index missing" defect: SPEC/Files were never auto-built when called from a non-TUI session, and Issue indexing was duplicated per-Worktree because all collections shared `$WORKTREE/.gwt/index/`.

This phase consolidates all index storage under `~/.gwt/index/<repo-hash>/`, separates Issue (Worktree-independent, 15-min TTL background refresh) from SPEC/File (Worktree-scoped, watcher-driven), upgrades the embedding model to `intfloat/multilingual-e5-base`, and makes the runner self-bootstrapping.

1. **Phase 5a — Failing tests (TDD RED)**
   - Add Python pytest suite under `crates/gwt-core/runtime/tests/`: `test_e5_prefix.py`, `test_auto_build_fallback.py`, `test_manifest_diff.py`, `test_flock.py`, `test_issue_ttl.py`, `test_repo_layout.py`
   - Add Rust integration tests under `crates/gwt-core/tests/`: `repo_hash.rs`, `worktree_hash.rs`, `index_paths.rs`, `watcher_debounce.rs`, `worktree_gc.rs`, `issue_refresh.rs`, `index_runner_spawn.rs`
   - Verify all RED before any production code changes

2. **Phase 5b — Runner redesign**
   - Rewrite `crates/gwt-core/runtime/chroma_index_runner.py` to accept `--repo-hash`, `--worktree-hash`, `--scope {issues|specs|files|files-docs}`
   - Inject custom embedding function with e5 `passage:` / `query:` prefix handling
   - Add `portalocker`-based flock around DB directory (`<db>/.lock`)
   - Implement `--no-auto-build` opt-out, default behavior is auto-build on missing index with NDJSON progress on stderr
   - Implement `manifest.json` based incremental indexing for `index-files` / `index-specs`
   - Track Issue TTL via `<issues>/meta.json::last_full_refresh`; add `--respect-ttl` flag
   - Update `crates/gwt-core/runtime/project_index_requirements.txt` to add `sentence-transformers` and `portalocker`
   - Drive Python pytest suite to GREEN

3. **Phase 5c — Rust helpers**
   - New `crates/gwt-core/src/repo_hash.rs`: `git remote get-url origin` → normalize → SHA256[:16]
   - New `crates/gwt-core/src/worktree_hash.rs`: canonicalize absolute path → SHA256[:16]
   - Extend `crates/gwt-core/src/paths.rs` with `gwt_index_db_path(repo_hash, worktree_hash, Scope)`
   - New `crates/gwt-core/src/index/mod.rs` module containing `runtime.rs`, `manifest.rs`, `watcher.rs`
   - Job helpers in `crates/gwt-core/src/runtime.rs` (or new `crates/gwt-core/src/index/runtime.rs`): `IssueIndexJob`, `WorktreeIndexJob`, `WorktreeWatcherJob` (tokio task spawn)
   - Drive Rust unit/integration tests for these modules to GREEN

4. **Phase 5d — Watcher**
   - Implement `crates/gwt-core/src/index/watcher.rs` with `notify` crate, 2 s debounce, 100-file batch, `.gitignore` filtering
   - Spawn shape: `start_watcher(repo_hash, wt_hash, worktree_path) -> WatcherHandle`
   - Watcher batches feed into `runner index-{specs,files} --mode incremental`
   - Drive `watcher_debounce.rs` to GREEN

5. **Phase 5e — TUI integration**
   - Modify `crates/gwt-tui/src/main.rs` startup sequence (around the existing `load_initial_data` call): spawn tokio tasks for `reconcile_repo`, `refresh_issues_if_stale`, and per-worktree `start_watcher`
   - Modify `crates/gwt-tui/src/app.rs::spawn_pty_for_session` (≈line 2875) to ensure watcher exists for the spawned worktree and to kick an integrity check
   - Add `remove_worktree_index(repo_hash, wt_hash)` to the Worktree remove handler
   - Manual quickstart verification (`specs/SPEC-10/quickstart.md`)

6. **Phase 5f — Skill documentation + agent env**
   - Update `.claude/skills/gwt-search/SKILL.md`, `gwt-spec-search`, `gwt-issue-search`, `gwt-project-search` to:
     - Document `--repo-hash` / `--worktree-hash` / `--scope` arguments
     - Describe `GWT_REPO_HASH` and `GWT_WORKTREE_HASH` environment variables
     - Remove the now-incorrect "Index not found" error documentation; describe the auto-build fallback contract
   - Modify `crates/gwt-agent/src/launch.rs` to export `GWT_REPO_HASH` and `GWT_WORKTREE_HASH` to spawned panes

7. **Phase 5g — Verification & PR**
   - `cargo test -p gwt-core -p gwt-tui` (incl. `--ignored` for e2e)
   - `pytest -q crates/gwt-core/runtime/tests`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo fmt`
   - Manual e2e per quickstart
   - Single PR `bugfix/not-work-index → develop`

## Architecture Boundaries

- `crates/gwt-core/src/index/` — new submodule owning index lifecycle (manifest, watcher, runtime job spawning, hash computation re-exports)
- `crates/gwt-core/runtime/chroma_index_runner.py` — sole ChromaDB writer/reader; Rust never touches sqlite directly
- DB physical location: `~/.gwt/index/<repo-hash>/{issues,worktrees/<wt-hash>/{specs,files}}` with `.lock` sentinel files
- TUI process owns all watchers; non-TUI sessions rely on the runner's auto-build fallback + mtime stat reconciliation

## Dependencies

- gwt-git: Repository::detect_type()
- gwt-core: process::run_command() for git clone
- gwt-core (new): `sha2`, `notify`, `notify-debouncer-mini`, `fs2` crates
- Python runtime (new): `sentence-transformers`, `portalocker`

## Verification

1. `cargo test -p gwt-tui` — all pass
2. `cargo test -p gwt-git` — all pass
3. `cargo test -p gwt-core` — runtime bootstrap tests pass + Phase 5 unit/integration tests pass
4. `cargo test -p gwt-core --tests -- --ignored` — e2e index runner spawn tests pass (CI uses HF model cache)
5. `pytest -q crates/gwt-core/runtime/tests` — all Phase 5 Python tests pass
6. E2E: initialization screen snapshot, clone flow, bare migration screen, runtime self-repair
7. E2E: legacy `.gwt/index` auto-removal, runner cold-start auto-build, watcher live-reflect, worktree-remove index cleanup, async issue refresh non-blocking
