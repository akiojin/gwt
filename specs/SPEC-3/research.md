# Research: SPEC-3 - Agent Management

## Scope Snapshot
- Canonical scope: Agent detection, launch wizard flows, custom agent definitions, version cache refresh, and session conversion.
- Current status: `in-progress` / `Implementation`.
- Task progress: `33/33` checked in `tasks.md`.
- Notes: This SPEC now treats session conversion as a metadata-driven agent switch, matching the implemented code and tests.

## Decisions
- Keep agent detection, launch wizard, and cache refresh inside one management domain because they share startup orchestration.
- Record session conversion according to the implemented metadata-swap flow rather than the older PTY-relaunch wording.
- Keep completion claims gated by acceptance and reviewer evidence even after all tracked tasks are checked.

## Open Questions
- Confirm whether any additional startup refresh telemetry is needed before closing the cache work.
- Decide what manual reviewer evidence is required before moving this SPEC from implementation to closed.
