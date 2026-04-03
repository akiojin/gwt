# Progress: SPEC-9 - Infrastructure

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `78/82` checked in `tasks.md`
- Artifact refresh: `2026-04-03T10:15:00Z`

## Done
- Supporting artifacts now cover the full infrastructure umbrella instead of only the core four files.
- Progress notes distinguish the more advanced hooks work from the still-open Docker and release tasks.
- Checklists are now present for future completion-gate reconciliation.
- Embedded skill registry tests and builtin catalog registration are in place, and the TUI model now initializes the builtin registry at startup.
- DockerProgress now has explicit stage-status and failure-render tests, so the overlay itself is no longer speculative.
- DockerProgress can now be driven by external `SetStage/Hide` messages from `app.rs`, which gives the future Docker event source a stable overlay contract without inventing a full manager first.
- The extended `gwt-pr-check` report now has deterministic parser coverage for CI, merge, and review state summaries.
- ServiceSelect now has focused coverage for multi-service listing, empty-state errors, and single-service auto-selection.
- PortSelect now has focused coverage for conflict detection, explicit acceptance flows, and auto-closing when all conflicts are resolved.
- Container lifecycle command execution is now testable without a real Docker daemon via a fake-binary seam in `gwt-docker`.
- Settings now expose a `Skills` category, render builtin embedded skills, and sync toggle state back into `SkillRegistry`.
- Branch detail `Overview` now exposes a Docker status area with container selection and synchronous start/stop/restart controls wired through `app.rs` with success/error feedback.

## Next
- Replace the stale `DockerManager async event stream` assumption with a real producer task that bridges `gwt-docker` sync APIs into background progress messages for `DockerProgress`.
- Reconcile the remaining completion-gate items and reviewer evidence now that only the producer-side Docker work is still open in this SPEC task list.
- Re-run infrastructure verification before any `Done` transition.
