# Progress: SPEC-1 - Terminal Emulation

## Progress
- Status: `open`
- Phase: `Implementation`
- Task progress: `13/17` checked in `tasks.md`
- Artifact refresh: `2026-04-03T10:15:00Z`

## Done
- Supporting artifacts now exist for planning, execution tracking, and review.
- URL detection and alt-screen verification tests exist at the renderer layer.
- `Ctrl+click` URL open and wrapped-URL support remain blocked by the missing end-to-end session surface foundation in `gwt-tui`.
- Acceptance and TDD checklists now reflect the blocked state instead of implying full delivery.

## Next
- Introduce a real vt100-backed session surface in `app.rs`/`model.rs` so PTY output, rendered cells, and URL regions are preserved per session.
- Only after that foundation exists, implement `Ctrl+click` URL open and wrapped-URL hit testing.
- Re-run SPEC analysis after the foundation task lands.
