# Analysis: SPEC-9 - Infrastructure — build distribution, Docker UI, embedded skills, hooks merge

## Analysis Report: SPEC-9

Status: BLOCKED BY MISSING FOUNDATION

## Blocking Items
- `T-005` still assumes an existing `DockerManager` async event stream, but the current repo only exposes synchronous `gwt-docker` APIs and a consumer-side `DockerProgress` contract.

## Checks
- Clarification completeness: no `[NEEDS CLARIFICATION]` markers remain in `spec.md`.
- Artifact completeness: `spec.md`, `plan.md`, `tasks.md`, supporting docs, `checklists/*`, `progress.md`, and `analysis.md` are present.
- Task traceability snapshot: `tasks.md` currently records `78/82` completed items.
- Notes: The embedded-skills scope now explicitly includes keeping bundled gwt-spec skills aligned with the local SPEC artifact model, including `analysis.md`.
- Notes: Builtin embedded skills are now initialized into the TUI model at startup, so the startup registration path is no longer speculative.
- Notes: DockerProgress now has focused coverage for explicit stage-status text and failure rendering, but the producer side is still open.
- Notes: The app layer now lazy-creates and tears down DockerProgress from external `SetStage/Hide` messages, so the missing piece is the producer side rather than the overlay contract itself.
- Notes: The settings panel now includes builtin embedded skills and syncs bool toggles back into `SkillRegistry`, so the remaining embedded-skills scope is no longer basic UI.
- Notes: ServiceSelect and PortSelect now have focused unit coverage for their primary decision paths, but they are not yet wired into the Docker orchestration path.
- Notes: Container lifecycle CLI execution now has deterministic tests via a fake-docker seam, and the Docker status area now exposes synchronous lifecycle controls with status-bar/error feedback.
- Notes: The extended `gwt-pr-check` report now has deterministic parser coverage for structured CI / merge / review output.
- Notes: Infrastructure delivery is still partial because the progress producer foundation has not been built yet.

## Next
- Replace the `DockerManager` wording in the remaining Docker plan/task language with a truthful producer task built on `gwt-docker`.
- Keep `T-005` open until a background worker or equivalent producer can drive `DockerProgress`.
- This report is a readiness gate, not a completion certificate.
