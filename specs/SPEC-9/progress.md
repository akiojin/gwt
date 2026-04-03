# Progress: SPEC-9 - Infrastructure

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `82/82` checked in `tasks.md`
- Artifact refresh: `2026-04-03T13:05:00Z`

## Done
- Supporting artifacts now cover the full infrastructure umbrella instead of
  only the core four files.
- Progress notes now reflect that the execution task list is fully checked and
  only completion-gate evidence remains.
- Checklists are now present for final completion-gate reconciliation.
- Embedded skill registry tests and builtin catalog registration are in place, and the TUI model now initializes the builtin registry at startup.
- DockerProgress now has explicit stage-status and failure-render tests, so the overlay itself is no longer speculative.
- DockerProgress can now be driven by a TUI-local background producer that wraps synchronous `gwt-docker` lifecycle calls and drains completion events on `Tick`.
- The extended `gwt-pr-check` report now has deterministic parser coverage for CI, merge, and review state summaries.
- ServiceSelect now has focused coverage for multi-service listing, empty-state errors, and single-service auto-selection.
- PortSelect now has focused coverage for conflict detection, explicit acceptance flows, and auto-closing when all conflicts are resolved.
- Container lifecycle command execution is now testable without a real Docker daemon via a fake-binary seam in `gwt-docker`.
- Settings now expose a `Skills` category, render builtin embedded skills, and sync toggle state back into `SkillRegistry`.
- Branch detail `Overview` now exposes a Docker status area with container selection and background lifecycle feedback wired through `app.rs`, including ready/failure transitions after the worker drains.

## Next
- Reconcile the remaining reviewer evidence and acceptance checklist items.
- Re-run infrastructure verification before any `Done` transition.
