# Research: SPEC-7 - Settings and Profiles

## Scope Snapshot
- Canonical scope: Configuration UI, profile management, environment variables, and voice-related settings.
- Current status: `in-progress` / `Implementation`.
- Task progress: `8/24` checked in `tasks.md`.
- Notes: The settings surface exists, but voice-setting parity and validation remain incomplete.

## Decisions
- Keep profiles, environment bindings, and voice configuration under one settings-management SPEC.
- Document missing voice fields and validation explicitly rather than assuming the partial UI is sufficient.
- Use supporting artifacts to keep configuration truth aligned with the live forms.

## Open Questions
- Confirm the final voice-settings field set and validation rules before treating the form complete.
- Decide whether profile switching and voice defaults should share one persistence path.
