# Progress: SPEC-9 - Infrastructure

## Progress
- Status: `in-progress`
- Phase: `Implementation`
- Task progress: `63/82` checked in `tasks.md`
- Artifact refresh: `2026-04-03T13:05:00Z`

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

## Next
- Connect the new DockerProgress message contract to a real Docker event producer, then finish the remaining status-area lifecycle gaps in the Docker UI flow.
- Finish any remaining embedded-skills execution surfaces beyond startup registration.
- Close release workflow validation and remaining hooks hardening items.
- Re-run infrastructure verification before updating task completion further.
