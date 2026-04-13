---
name: gwt-plan-spec
description: "Use when a SPEC exists and the next task is to generate or refresh implementation planning artifacts such as plan.md and tasks.md."
---

# gwt-plan-spec

Unified planning skill that translates a clarified `spec.md` into implementation-ready
artifacts through four sequential phases: technical context, architecture design,
task decomposition, and quality gate.

Covers planning, task decomposition, and pre-implementation quality-gate work
behind the visible `gwt-plan-spec` entrypoint.

## Invocation

- **With SPEC:** `gwt-plan-spec SPEC-<id>` — full pipeline from spec.md to quality gate
- **Lightweight:** `gwt-plan-spec` without a SPEC — produce a plan.md and tasks.md in the
  current working context (no spec.md required, skip traceability checks)
- **Standalone:** works independently of the visible SPEC flow owner

## Prerequisites

- If `spec.md` has critical `[NEEDS CLARIFICATION]` markers, use `gwt-discussion` first.
- If the target SPEC does not exist, use `gwt-discussion` to create it first.
- If planning artifacts already exist but are stale, update them — do not recreate blindly.

## Required inputs

- `spec.md` from the target SPEC directory (or a user-provided description in lightweight mode)
- `.gwt/memory/constitution.md`

## Required outputs

All artifacts are written to the SPEC directory (`specs/SPEC-<id>/`):

- `plan.md` — architecture, phases, constitution check
- `research.md` — unknowns, tradeoff decisions, external findings
- `data-model.md` — entities, shapes, lifecycle, invariants
- `quickstart.md` — minimum validation flow for reviewers and implementers
- `contracts/*` — interface or schema contracts (only when needed)
- `tasks.md` — executable work items with test-first ordering

Use the current user's language for generated artifact text, quality-gate
reports, and any user-facing planning summaries unless the artifact already has
an established language that must be preserved.

## Phase 1: Technical Context

Establish the implementation landscape before designing.

1. **Load source artifacts.** Read `spec.md` and `.gwt/memory/constitution.md`.
   Refuse to continue only when `spec.md` is missing or a user decision still blocks planning.

2. **Identify affected scope.** List files, modules, services, and external constraints
   that the work touches. Record assumptions explicitly.

3. **Run the constitution check.** Evaluate the work against every rule in
   `.gwt/memory/constitution.md`. If a rule is violated, either redesign or record the
   justification in `Complexity Tracking`.

4. **Answer the Required Plan Gates** from the constitution:
   - What files/modules are affected?
   - What constitution constraints apply?
   - Which risks or complexity additions are accepted, and why?
   - How will the acceptance scenarios be verified?

## Phase 2: Architecture Design (SDD)

Design the solution using Software Design Document methodology before decomposing tasks.

> See `references/sdd-design.md` for full methodology.

1. **Component design.** Identify new and modified components, their responsibilities,
   and ownership boundaries. Keep the design minimal — no unnecessary abstractions.

2. **Interface contracts.** Define public APIs, message formats, and protocol boundaries
   between components. Write stable contracts to `contracts/*` when interfaces cross
   module or crate boundaries.

3. **Data model.** Document entities, their shapes, lifecycle states, and invariants.
   Write to `data-model.md`.

4. **Sequence descriptions.** Describe key interaction flows in plain text — which
   component calls what, in what order, with what data. No diagram syntax required;
   clarity and completeness matter.

5. **Produce supporting artifacts.**
   - `research.md`: unknowns, tradeoff decisions, external findings
   - `quickstart.md`: minimum validation flow for reviewers and implementers

6. **Write `plan.md`.** Structure:
   - Summary
   - Technical Context (from Phase 1)
   - Constitution Check
   - Architecture Design (component, interface, data model, sequences)
   - Project Structure
   - Complexity Tracking
   - Phased Implementation

## Phase 3: Task Decomposition

Turn the architecture and plan into executable work items.

> See `references/task-decomposition.md` for full rules.

1. **Lay out phase order.** Canonical ordering:
   - **Setup** — tooling, configuration, scaffolding
   - **Foundational** — shared infrastructure, core types
   - **User Story phases** (`US1`, `US2`, ...) — story-specific implementation
   - **Polish / Cross-Cutting** — cleanup, documentation, integration tests

2. **Generate test-first tasks.** For each user story:
   - Add test/verification tasks before or alongside implementation tasks
   - Include contract, integration, and e2e coverage when the spec implies it

3. **Add implementation tasks.** Each task must include:
   - **Task ID** in `T-NNN` format
   - **`[P]` marker** when parallelizable (only when write scopes do not overlap)
   - **Linked user story ID** where applicable
   - **Exact file path or module**
   - **Concrete action** — no vague descriptions

4. **Validate traceability.**
   - Every user story has implementation and verification tasks
   - Every acceptance scenario has verification coverage
   - Every declared contract/data-model change has matching tasks

5. **Write `tasks.md`.**

## Phase 4: Quality Gate

Final readiness check before implementation. This is a pre-implementation gate only —
it does not certify that implementation is complete.

> See `references/quality-gate.md` for full check definitions.

### Mandatory checks

1. **Clarification completeness** — no critical `[NEEDS CLARIFICATION]` markers remain
2. **Spec completeness** — User Stories, Acceptance Scenarios, Edge Cases, Requirements,
   and Success Criteria exist
3. **Plan completeness** — Constitution Check exists, Technical Context and Phased
   Implementation are concrete
4. **Task traceability** — every user story has tasks, every acceptance scenario has
   verification coverage, every contract/data-model change has matching tasks
5. **Constitution alignment** — violations are either removed or explicitly tracked in
   Complexity Tracking

### Verdict

```text
## <Analysis Report in the current user's language>: SPEC-<id>

Status: CLEAR | AUTO-FIXABLE | NEEDS-DECISION

Blocking items:
- A1. <artifact gap>
- A2. <traceability gap>

Next:
- gwt-build-spec (on CLEAR)
- self-repair and rerun (on AUTO-FIXABLE)
- ask user for decision (on NEEDS-DECISION)
```

**Decision rule:**

- **CLEAR** — implementation may proceed through `gwt-build-spec`
- **AUTO-FIXABLE** — repair the artifact set in-place and rerun the quality gate
- **NEEDS-DECISION** — report points to the exact user decision or unresolved ambiguity

**Boundary:** CLEAR means artifacts are ready for execution. It does not mean the SPEC is
complete — completion requires post-implementation reconciliation in `gwt-build-spec`.

## Chain suggestion

On `CLEAR` verdict, suggest proceeding to `gwt-build-spec` for implementation.

## Lightweight mode

When invoked without a SPEC:

- Skip traceability checks (Phase 4 check 4)
- Skip constitution check if `.gwt/memory/constitution.md` is not found
- Produce `plan.md` and `tasks.md` in the current directory
- Omit supporting artifacts unless the user requests them

## Operations

Use `.claude/scripts/spec_artifact.py` for artifact persistence:

```bash
# Write plan.md
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:plan.md" \
  --body-file /tmp/plan.md

# Write research.md
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:research.md" \
  --body-file /tmp/research.md

# Write data-model.md
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:data-model.md" \
  --body-file /tmp/data-model.md

# Write quickstart.md
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:quickstart.md" \
  --body-file /tmp/quickstart.md

# Write tasks.md
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:tasks.md" \
  --body-file /tmp/tasks.md

# List artifacts
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --list
```
