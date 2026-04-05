# Research: SPEC-6 - Notification and Error Bus

## Scope Snapshot
- Canonical scope: Notification severity routing, status bar messaging, modal errors, queue handling, and structured logs.
- Current status: `in-progress` / `Implementation`.
- Task progress: `14/44` checked in `tasks.md`.
- Notes: The core bus and log pieces exist, but the TUI routing story remains only partially completed.

## Decisions
- Keep status-bar info, warning persistence, modal errors, and structured logs inside one notification domain.
- Describe partial routing as partial routing, even when the backend bus already exists.
- Use the checklists to distinguish broad regression evidence from missing UI-level notification verification.

## Open Questions
- Confirm the final UX for `Info` auto-dismiss and `Warn` persistence before closing the routing work.
- Decide how much of the error queue lifecycle must be visible in logs versus modal state.
