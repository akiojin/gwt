# Research: SPEC-3 - Agent Management

## Scope Snapshot
- Canonical scope: Agent detection, launch wizard flows, custom agent definitions, version cache refresh, and session conversion.
- Current status: `in-progress` / `Implementation`.
- Task progress: `32/33` checked in `tasks.md`.
- Notes: This SPEC is near completion in task count, but the progress artifacts must stay honest about the remaining PTY-relaunch reconciliation.

## Decisions
- Keep agent detection, launch wizard, and cache refresh inside one management domain because they share startup orchestration.
- Record session conversion as implemented only to the extent the current metadata-swap flow actually exists.
- Do not overstate completion until the remaining PTY replacement expectation is either delivered or explicitly narrowed.

## Open Questions
- Decide whether the last unchecked task requires a real PTY relaunch or a spec adjustment to match the adopted flow.
- Confirm whether any additional startup refresh telemetry is needed before closing the cache work.
