---
name: gwt-spec-design
description: "Use when a legacy prompt or internal handoff refers to gwt-spec-design. Prefer gwt-design-spec as the visible SPEC design entrypoint."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[rough idea or request | --deepen SPEC-N]"
---

# gwt-spec-design

Unified design skill that takes a rough idea through domain discovery to a
planning-ready SPEC. Absorbs brainstorm, register, clarify, and deepen into a
single five-phase pipeline.

Visible owner: `gwt-design-spec`.

## Standalone usage

| Invocation | Behavior |
|---|---|
| `gwt-spec-design` | Run Phase 1-4 sequentially (intake through clarification) |
| `gwt-design <rough idea>` | Same, seeded with the given idea |
| `gwt-design --deepen SPEC-N` | Run Phase 5 only on an existing SPEC |

## Phase overview

| Phase | Name | Source | Purpose |
|-------|------|--------|---------|
| 1 | Intake | brainstorm | Search, interview, route |
| 2 | Domain Discovery | NEW (DDD) | Bounded Contexts, entities, Ubiquitous Language |
| 3 | Registration | register | Create SPEC directory and seed spec.md |
| 4 | Clarification | clarify | Resolve [NEEDS CLARIFICATION], lock scenarios |
| 5 | Deepening | deepen | Challenge assumptions, explore alternatives (optional) |

---

## Phase 1: Intake

> Reference: `references/intake.md`

### Mandatory preflight search

Before any creation or routing decision:

1. Use `gwt-issue-search` with at least 2 semantic queries from the request.
2. Use `gwt-spec-search` with at least 2 semantic queries from the request.
3. If unclear, also run:

```bash
python3 ".claude/scripts/spec_artifact.py" --repo "." --list-all
```

1. Prefer an existing canonical SPEC or Issue when a clear owner exists.
2. Only continue toward new work when the duplicate check is clean.

Do not skip search because the idea "sounds new".

### Interview

Interview is mandatory. One question at a time. Wait for the answer.

Priority order:

1. User goal / why now
2. Target user or actor
3. Current pain or missing capability
4. Scope boundary and explicit non-goals
5. Success condition for the first slice
6. Integration target or owning subsystem

Rules:

- Prefer multiple-choice framing when it reduces ambiguity.
- Ask only what materially changes routing, scope, behavior, or testability.
- Do not ask questions already answered by the repo or search results.
- Do not answer product or scope decisions on the user's behalf.
- If the request is too broad for one SPEC, stop and decompose first.

### Intake memo

Maintain in conversation context (not as a file):

```markdown
## Intake Memo
- Request:
- Why now:
- Users / actors:
- In scope:
- Out of scope:
- Constraints:
- Success signal:
- Open questions:
- Candidate owner:
```

### Routing decision

After enough signal, produce:

```text
## Registration Decision
**Duplicate Check:** CLEAN | EXISTING-ISSUE | EXISTING-SPEC | AMBIGUOUS
**Chosen Path:** EXISTING-ISSUE | EXISTING-SPEC | NEW-SPEC | ISSUE | TOO-BROAD-SPLIT-FIRST
**Action:** <next step>

### Intake Memo
- <final bullets>

### Candidates
- #<number> <title> [issue] - <match reason>
- SPEC-<id> <title> [spec] - <match reason>
```

Routing rules:

| Path | Action |
|---|---|
| EXISTING-SPEC | Skip to Phase 4 (clarify the owning SPEC) |
| EXISTING-ISSUE | Hand off to `gwt-fix-issue` |
| NEW-SPEC | Continue to Phase 2 |
| ISSUE | Hand off to `gwt-register-issue` |
| TOO-BROAD-SPLIT-FIRST | Ask user to pick first slice, restart Phase 1 |
| NO-ACTION | End with documented rationale (no SPEC or Issue needed) |

Do not ask "should I proceed?" after the decision.

### Skip condition

If invoked from `gwt-spec-brainstorm` with an Intake Memo already
prepared, skip the search and interview steps in Phase 1. Validate the
memo against the search results brainstorm already ran, then proceed
directly to Phase 2 (Domain Discovery). This avoids duplicate search
and interview when transitioning from brainstorm to design.

---

## Phase 2: Domain Discovery (DDD)

> Reference: `references/ddd-modeling.md`

Apply Domain-Driven Design modeling to the intake before registration.

### Step 2.1: Bounded Context identification

- Identify which Bounded Contexts the feature touches.
- Map the feature to existing BCs in the codebase (`gwt-core`, `gwt-tui`, etc.).
- If a new BC is needed, name it and define its responsibility boundary.

### Step 2.2: Entity and aggregate mapping

- List key entities and value objects the feature introduces or modifies.
- Identify the aggregate root for each cluster of entities.
- Map relationships (owns, references, depends-on).

### Step 2.3: Ubiquitous Language

- Extract domain terms from the intake memo.
- Define each term precisely (one sentence).
- Flag terms that conflict with existing codebase terminology.
- Record the glossary as a `## Ubiquitous Language` section for spec.md.

### Step 2.4: BC boundary check (granularity gate)

Use the DDD model to validate SPEC granularity:

- A SPEC should map to **one primary Bounded Context**.
- If the feature spans multiple BCs, consider splitting into per-BC SPECs.
- Cross-BC interactions should be documented as integration points, not merged scope.

Also apply the SPEC vs Issue decision:

| Criteria | SPEC | Issue |
|---|---|---|
| New user-facing functionality | Yes | -- |
| Architecture or design decisions | Yes | -- |
| Bug fix | -- | Yes (link to parent SPEC) |
| One-off chore | -- | Yes |

SPEC scope is determined by feature cohesion, not task count.
Implementation phasing is handled by `gwt-plan-spec`.

### DDD output

Record the model summary in conversation context for use in Phase 3:

```markdown
## Domain Model Summary
### Bounded Contexts
- <BC name>: <responsibility>
### Entities
- <Entity>: <description> (BC: <name>)
### Ubiquitous Language
- <Term>: <definition>
### Integration Points
- <BC-A> -> <BC-B>: <interaction>
```

---

## Phase 3: Registration

> Reference: `references/registration.md`

### Language contract

Use the current user's language for the SPEC title, seeded artifact content,
clarification text, and any user-facing summaries generated during this
workflow. Keep the `gwt-spec:` prefix stable, but write the remainder of the
title in the current user's language rather than forcing English.

### Create SPEC directory

Determine next SPEC ID, then:

```bash
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." --create --title "gwt-spec: <description in the current user's language>"
```

Title convention: always use `gwt-spec:` prefix with a short action-oriented
summary in the current user's language.

### Directory structure

```text
specs/SPEC-{id}/
  metadata.json
  spec.md
```

### Seed spec.md

Write the initial spec.md with this template, populated from intake memo and
domain model. The headings below are illustrative only; render the actual
artifact in the current user's language:

```markdown
# Feature Specification: <title>

## Background

...

## Ubiquitous Language

- <Term>: <definition>

## User Stories

### User Story 1 - <title> (Priority: P1)

...

## Acceptance Scenarios

1. Given ...

## Edge Cases

- ...

## Functional Requirements

- FR-001 ...

## Non-Functional Requirements

- NFR-001 ...

## Success Criteria

- SC-001 ...
```

Use `[NEEDS CLARIFICATION: <question>]` for unknowns. Do not guess.

Upload via:

```bash
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --upsert \
  --artifact "doc:spec.md" --body-file /tmp/spec.md
```

Do not create plan.md or tasks.md here.

---

## Phase 4: Clarification

> Reference: `references/clarification.md`

### Find gaps

1. Scan for `[NEEDS CLARIFICATION]` markers.
2. Check for missing user stories, weak acceptance scenarios, vague requirements,
   unstated edge cases.

### Resolve and ask

- Fill obvious gaps from source Issue, existing code, and surrounding artifacts.
- Present remaining high-impact questions (max 5) to the user.
- **STOP and wait for answers.** Do not answer on the user's behalf.
- Use numbered options with trade-off explanations.

### Standard question checklist

All SPECs:

- v1 scope boundary (included vs excluded)
- Integration with existing features
- Performance / memory requirements
- Error handling policy

UI component: data storage, default size, relationship to other components,
library selection, out-of-viewport behavior.

Backend: API design, data format, concurrency, migration policy.

Refactoring: impact scope, data migration, breaking changes.

### Update spec.md

- Replace resolved markers with user answers.
- Add `## Clarification Log` section recording Q&A.
- Tighten user stories and acceptance scenarios.

### Planning-ready exit criteria

- No critical `[NEEDS CLARIFICATION]` markers remain.
- All questions answered by the user (not by agent assumption).
- Each user story has at least one acceptance scenario.
- Edge cases are explicit.
- Functional and success criteria are testable.

Report:

```text
## Clarification Report: SPEC-<id>
Resolved: <N>
Remaining blockers: <M>
Next: gwt-plan
```

---

## Phase 5: Deepening (optional)

> Reference: `references/deepening.md`

Invoke with `gwt-design --deepen SPEC-N` or when the user requests deeper
exploration after clarification.

### Phase 5.1: Analysis report

Read all SPEC artifacts and produce a categorized report:

**Spec-level:**

- [ASSUMPTION] premises that could be wrong
- [EDGE CASE] failure modes not addressed
- [ALTERNATIVE] design choices not compared
- [VAGUE] requirements lacking measurable criteria
- [USER STORY GAP] actors or flows not covered

**Task-level (if tasks.md exists):**

- [COARSE] tasks that should be split
- [MISSING TEST] implementation without test-first
- [DEPENDENCY] implicit dependencies
- [UNCLEAR SCOPE] ambiguous expected output

Ask the user which points to deep-dive on.

### Phase 5.2: Interactive deep-dive

For each selected point:

1. Quote the relevant artifact section.
2. Ask one targeted question.
3. Wait for the answer.
4. Propose a concrete artifact update.
5. Apply after user confirmation.

Question rules: ask questions that change implementation, present trade-offs
with consequences, challenge obvious answers, never answer design questions
for the user.

---

## Chain suggestion

On completion of Phase 4 (or Phase 5), suggest:

```text
SPEC-<id> is planning-ready. Run `gwt-plan-spec` to generate plan.md and
supporting artifacts.
```

## Operations reference

All artifact operations use the shared helper:

```bash
# List all SPECs
python3 ".claude/scripts/spec_artifact.py" --repo "." --list-all

# Create new SPEC
python3 ".claude/scripts/spec_artifact.py" --repo "." --create --title "gwt-spec: ..."

# Read artifact
python3 ".claude/scripts/spec_artifact.py" --repo "." --spec "<id>" --get --artifact "doc:spec.md"

# List SPEC artifacts
python3 ".claude/scripts/spec_artifact.py" --repo "." --spec "<id>" --list

# Upsert artifact
python3 ".claude/scripts/spec_artifact.py" --repo "." --spec "<id>" --upsert --artifact "doc:spec.md" --body-file /tmp/spec.md
```

Search tools:

- `gwt-issue-search` for GitHub Issues
- `gwt-spec-search` for local SPEC files
- `gwt-project-search` for source files
