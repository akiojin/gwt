# Clarification Reference

Detailed logic for Phase 4 of gwt-design.

## Purpose

Turn a draft spec.md into a planning-ready specification by resolving all
`[NEEDS CLARIFICATION]` markers and tightening user stories and acceptance
scenarios.

## Artifact contract

This phase works on:

```text
specs/SPEC-{id}/spec.md
```

The file must contain these sections:

- Background
- Ubiquitous Language (added in Phase 2/3)
- User Stories
- Acceptance Scenarios
- Edge Cases
- Functional Requirements
- Non-Functional Requirements
- Success Criteria

## Workflow

### Step 1: Read current spec.md

```bash
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --get --artifact "doc:spec.md"
```

If spec.md does not exist, return to Phase 3 (registration) first.

### Step 2: Find clarification gaps

Priority order:

1. Explicit `[NEEDS CLARIFICATION]` markers.
2. Missing user stories (actors or flows not covered).
3. Weak acceptance scenarios (no concrete Given/When/Then).
4. Vague requirements (no measurable criteria).
5. Unstated edge cases (failure modes, boundaries).

### Step 3: Resolve what is knowable

Fill obvious gaps from:

- The source Issue or request.
- Existing comments and discussion.
- Current codebase implementation.
- Surrounding SPEC artifacts.

Do not guess at product or design decisions.

### Step 4: Present questions to the user

Rules:

- Ask at most 5 questions, ordered by implementation impact.
- Present each question with numbered options and trade-off explanations.
- **STOP and wait for answers.**
- **Never answer clarification questions on the user's behalf.**
- Use the standard question checklist below to avoid missing critical questions.

### Step 5: Update spec.md

After receiving answers:

- Replace resolved `[NEEDS CLARIFICATION]` markers with the user's actual answers.
- Add a `## Clarification Log` section recording each Q&A pair for traceability.
- Tighten user stories and acceptance scenarios while preserving intent.
- Keep unresolved blockers explicit.

Upload:

```bash
python3 ".claude/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --upsert \
  --artifact "doc:spec.md" --body-file /tmp/spec.md
```

### Step 6: Decide next step

- If a real product or scope decision remains: stop with a blocker summary.
- If remaining gaps are implementation-local: resolve them here.
- If planning-ready: proceed to chain suggestion (gwt-plan).

## Standard question checklist

### All SPECs

- [ ] v1 scope boundary: what is included, what is explicitly excluded?
- [ ] Integration with existing features: how does this connect?
- [ ] Performance / memory requirements: specific thresholds?
- [ ] Error handling policy: what happens on failure?

### New UI component / tile

- [ ] Data storage location and persistence method
- [ ] Default size / layout constraints
- [ ] Relationship with other components (edge, linked, independent)
- [ ] Library / dependency selection (present trade-offs)
- [ ] Behavior when outside viewport (mount / unmount policy)

### Backend feature

- [ ] API interface design
- [ ] Data format / serialization
- [ ] Concurrency / exclusion control
- [ ] Migration policy for existing data

### Refactoring

- [ ] Impact scope (number of files, tests)
- [ ] Persisted data migration policy
- [ ] Breaking changes

## Planning-ready exit criteria

All of the following must be true:

- No critical `[NEEDS CLARIFICATION]` markers remain.
- All clarification questions have been answered by the user (not by agent).
- Each user story has at least one acceptance scenario.
- Edge cases are explicit where implementation could diverge.
- Functional requirements and success criteria are testable.
- Ubiquitous Language section is consistent with user stories and FRs.

## Output format

```text
## Clarification Report: SPEC-<id>

Resolved: <N> markers
Remaining blockers: <M>
Planning-ready: YES | NO (reason)
Next: gwt-plan | ask follow-up clarification
```
