# Research: SPEC-8 - Input Extensions

## Scope Snapshot
- Canonical scope: Voice input, normal terminal paste, and AI-assisted branch naming.
- Current status: `in-progress` / `Implementation`.
- Task progress: `49/49` checked in `tasks.md`, with Phase 2 refreshed from file paste to terminal paste.
- Notes: This SPEC has meaningful partial implementation, but backend completion and manual verification still lag the task list.

## Decisions
- Keep voice capture, terminal paste, and branch naming together because they all extend non-typed input into terminal workflows.
- Treat the earlier clipboard file-path hotkey as a superseded experiment rather than current product surface.
- Treat helper-library wiring and keybinding registration as partial progress, not end-to-end completion.
- Do not mark the feature complete until PTY injection, backend behavior, and reviewer flows all agree.

## Open Questions
- Confirm what counts as a complete voice backend: local model, external provider, or pluggable abstraction.
- Decide how branch-suggestion failures should degrade during interactive workflows.
