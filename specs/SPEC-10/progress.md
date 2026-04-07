# Progress: SPEC-10 - Project Workspace

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `95/96` checked in `tasks.md` (Phase 8: 49/50; T-IDX-049 manual e2e pending user verification)
- Artifact refresh: `2026-04-07T20:00:00Z`

## Done
- The missing support artifacts for this near-complete SPEC are now present.
- Progress notes now reflect that initialization and migration work are largely delivered.
- Coverage and metadata closure tasks are now checked in `tasks.md`.
- Completion tracking now distinguishes finished execution from final reviewer
  evidence.
- Shared project-index runtime repair now runs both after clone and on normal startup.
- Managed `chroma-venv` recovery is covered by focused `gwt-core` runtime tests.
- Phase 6 hardened bootstrap Python detection so launcher candidates are execution-probed before `chroma-venv` creation.
- Startup and clone-completion warnings now preserve Python install guidance instead of surfacing only raw runtime setup failures.
- Phase 7 completed the review follow-up: working Store/launcher Python entrypoints are accepted, future `python3.x` binaries can be discovered from `PATH`, runtime failures preserve probe detail, and startup/clone warnings share stable project-index classification.
- Broad verification reran successfully across `cargo fmt -- --check`, `cargo test -p gwt-core -p gwt-git -p gwt-tui`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo build -p gwt-tui`.

## Phase 8 Reopening (2026-04-07)

The gwt-search skill family was found to be unable to recover from a missing index: the runner returned `Index not found` and neither the runner nor the SKILL.md provided any auto-recovery path. Combined with several latent design problems (no real file watcher despite documentation, Issue index duplicated per Worktree, English-only embedding model, no flock), this triggered a Phase 8 reopening of SPEC-10 to consolidate index lifecycle management.

Phase 8 introduces FR-017 through FR-029 and SC-011 through SC-016. The DB physical layout moves from `$WORKTREE/.gwt/index/` to `~/.gwt/index/<repo-hash>/`, separating Worktree-independent Issue data from Worktree-scoped SPEC/Files data. The runner gains an auto-build fallback so non-TUI sessions remain functional. The TUI gains a resident `notify` watcher per open Worktree and a non-blocking 15-minute Issue refresh on startup. The embedding model is upgraded from `all-MiniLM-L6-v2` to `intfloat/multilingual-e5-base` to handle Japanese SPEC content.

## Phase 8 Implementation Done (2026-04-07)

Phase 8 implementation is complete. PR #1912 (`bugfix/not-work-index → develop`) carries
the full change set. All Phase 8a–8g tasks except T-IDX-049 (manual user e2e) are GREEN:

- `cargo build --workspace` ✅
- `cargo test -p gwt-core` (100 tests) ✅
- `cargo test -p gwt-tui` (48 tests) ✅
- `cargo clippy --all-targets --all-features -- -D warnings` ✅
- `cargo fmt --all -- --check` ✅
- `pytest crates/gwt-core/runtime/tests/` (40 tests) ✅
- `bunx commitlint --from HEAD~1 --to HEAD` ✅

Pre-existing PTY tests (`gwt-terminal::pty::*`) still fail under macOS sandbox; verified
this is unrelated to Phase 8 by reproducing on `git stash`-clean main.

## Next
- T-IDX-049: user-side manual e2e per `quickstart.md` Phase 8 reviewer flow.
- Optional follow-up: enable `cargo test -- --ignored` in CI with HF model cache warm-up
  so the e2e tests in `tests/index_runner_spawn.rs` execute on every PR.
