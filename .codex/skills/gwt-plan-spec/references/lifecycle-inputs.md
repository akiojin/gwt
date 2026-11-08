# gwt-plan-spec

Unified planning skill that translates a clarified `spec.md` into implementation-ready
artifacts through four sequential phases: technical context, architecture design,
task decomposition, and quality gate.

Covers planning, task decomposition, and pre-implementation quality-gate work
behind the visible `gwt-plan-spec` entrypoint.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Invocation

- **With SPEC:** `gwt-plan-spec SPEC-<id>` — full pipeline from spec.md to quality gate
- **Lightweight:** `gwt-plan-spec` without a SPEC — produce a plan.md and tasks.md in the
  current working context (no spec.md required, skip traceability checks)
- **Standalone:** works independently of the visible SPEC flow owner

## Exit CLI (Stop-block contract)

SPEC-1935 FR-014q routes Stop through `skill-plan-spec-stop-check`, which reads
`.gwt/skill-state/plan-spec.json` and blocks Stop while the skill is active.
Register the skill lifecycle explicitly with the exit CLI:

- JSON operation `plan.start` with `params.spec:<n>` at the beginning of the skill invocation (`params.spec:0` for lightweight mode)
- JSON operation `plan.phase` with `params.spec:<n>` (or `params.spec:0` for lightweight mode) and
  `params.label:"plan-draft"|"tasks-draft"|"quality-gate"` at each internal milestone (logging only; does not affect blocking)
- JSON operation `plan.complete` with `params.spec:<n>` (or `params.spec:0` for lightweight mode) once the quality gate verdict is `CLEAR` and planning artifacts are written
- JSON operation `plan.abort` with `params.spec:<n>` (or `params.spec:0` for lightweight mode) and `params.reason` when planning cannot proceed (e.g. spec gap discovered mid-planning)

The Stop-block handler honours Claude Code / Codex's built-in
`stop_hook_active` flag, so forced continuation is capped at one per Stop
cycle and the skill will never infinitely loop even if the exit CLI is
skipped.

## Prerequisites

- If `spec.md` has critical `[NEEDS CLARIFICATION]` markers, use `gwt-discussion` first.
- If the target SPEC does not exist, use `gwt-discussion` to create it first.
- If planning artifacts already exist but are stale, update them — do not recreate blindly.

## Required inputs

- `spec.md` from the target SPEC directory (or a user-provided description in lightweight mode)
- Applicable `.gwt/memory/constitution.md` guidance, when present

## Required outputs

Write plan/tasks to the SPEC’s GitHub Issue sections; include supporting artifacts when needed by this scope:

- `plan.md` — architecture, phases, constitution check
- `research.md` — unknowns, tradeoff decisions, external findings
- `data-model.md` — entities, shapes, lifecycle, invariants
- `quickstart.md` — minimum validation flow for reviewers and implementers
- `contracts/*` — interface or schema contracts (only when needed)
- `tasks.md` — executable work items with test-first ordering

Use the current user's language for generated artifact text, quality-gate
reports, and any user-facing planning summaries unless the artifact already has
an established language that must be preserved.
