# Progress: SPEC-10 - Project Workspace

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `46/96` checked in `tasks.md` (Phase 8 added: 50 new tasks)
- Artifact refresh: `2026-04-07T13:30:00Z`

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

## Next
- Drive Phase 8 RED → GREEN per `tasks.md` Phase 8 sub-phases (8a–8g).
- Reconcile the acceptance checklist and final reviewer evidence before
  changing SPEC status.
