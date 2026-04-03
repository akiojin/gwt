# Research: SPEC-9 - Infrastructure

## Scope Snapshot
- Canonical scope: Build distribution, Docker-oriented UI flows, embedded skills, and hooks merge behavior.
- Current status: `in-progress` / `Implementation`.
- Task progress: `29/77` checked in `tasks.md`.
- Notes: Infrastructure work is broad and partially implemented; hooks work is farther along than Docker, embedded skills, or release completion.

## Decisions
- Group Docker UI, embedded skills, release packaging, and hooks merge here because they are support infrastructure rather than core shell UX.
- Track hooks hardening separately inside progress notes so it does not hide the unfinished Docker and release work.
- Keep the quickstart focused on reviewer validation points rather than promising a fully finished infrastructure stack.

## Open Questions
- Confirm the minimum release workflow evidence required before this SPEC can move toward completion.
- Decide whether Docker UI and embedded skills should remain in one umbrella SPEC if their delivery timing keeps diverging.
