# Research: SPEC-8 - Input Extensions

## Scope Snapshot
- Canonical scope: Voice input, clipboard-driven file paste, and AI-assisted branch naming.
- Current status: `in-progress` / `Implementation`.
- Task progress: `22/49` checked in `tasks.md`.
- Notes: This SPEC has meaningful partial implementation, but backend completion and manual verification still lag the task list.

## Decisions
- Keep voice capture, file paste, and branch naming together because they all extend non-typed input into terminal workflows.
- Treat helper-library wiring and keybinding registration as partial progress, not end-to-end completion.
- Do not mark the feature complete until PTY injection, backend behavior, and reviewer flows all agree.

## Open Questions
- Confirm what counts as a complete voice backend: local model, external provider, or pluggable abstraction.
- Decide how branch-suggestion failures should degrade during interactive workflows.
