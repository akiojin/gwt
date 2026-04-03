# Research: SPEC-3 - Agent Management

## Scope Snapshot
- Canonical scope: Agent detection, launch wizard flows, custom agent definitions, version cache refresh, and session conversion.
- Current status: `in-progress` / `Implementation`.
- Task progress: `38/38` checked in `tasks.md`.
- Notes: This SPEC now treats session conversion as a metadata-driven agent
  switch, matching the implemented code and tests.
- Notes: Version selection is now a dedicated wizard step with installed,
  `latest`, and cached semver options, and launch confirmation materializes a
  persisted agent session.

## Decisions
- Keep agent detection, launch wizard, and cache refresh inside one management domain because they share startup orchestration.
- Record session conversion according to the implemented metadata-swap flow rather than the older PTY-relaunch wording.
- Separate model selection from version selection so UI labels do not leak
  into CLI flags.
- Materialize the confirmed launch via pending session state after the wizard
  closes, which avoids borrow conflicts in the app layer.
- Keep completion claims gated by acceptance and reviewer evidence even after all tracked tasks are checked.

## Open Questions
- Confirm whether any additional startup refresh telemetry is needed before closing the cache work.
- Decide what manual reviewer evidence is required before moving this SPEC from implementation to closed.
