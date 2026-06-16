# Clarification Reference

Detailed logic for the clarification phase of gwt-discussion.

## Purpose

Turn a draft spec.md into a planning-ready specification by resolving all
`[NEEDS CLARIFICATION]` markers and tightening user stories and acceptance
scenarios.

## Artifact contract

This phase works on:

The `spec` section of GitHub Issue #<issue-number>, read with JSON operation `issue.spec.section`.

The `spec` section includes this structure:

- State
- Background
- Ubiquitous language (added in Phase 2/3)
- User stories
- Acceptance scenarios
- Edge cases
- Functional requirements
- Non-functional requirements
- Success criteria

## Workflow

### Step 1: Read current spec.md

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.section","params":{"number":123,"section":"spec"}}
JSON
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

- Present each question with numbered options and trade-off explanations.
- **STOP and wait for answers.**
- **Never answer clarification questions on the user's behalf.**
- Use the standard question checklist below to avoid missing critical questions.
- Do not stop because a fixed question count was reached.
- Continue asking follow-up clarification questions until the planning-ready
  exit criteria are satisfied or a real blocker remains.
- Planning-ready requires covering the applicable categories from the standard
  question checklist.
- Update the active proposal's `Question Ledger` after each answer and leave
  `Depth Gate: open` while a category can still change implementation,
  ownership, routing, failure handling, migration, or verification.

### Step 5: Update spec.md

After receiving answers:

- Replace resolved `[NEEDS CLARIFICATION]` markers with the user's actual answers.
- Add a `## Clarification Log` section recording each Q&A pair for traceability.
- Tighten user stories and acceptance scenarios while preserving intent.
- Keep unresolved blockers explicit.

Upload:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"issue.spec.edit","params":{"number":123,"section":"spec","body":"<updated spec body>"}}
JSON
```

### Step 6: Decide next step

- If a real product or scope decision remains: stop with a blocker summary.
- If remaining gaps are implementation-local: resolve them here.
- If planning-ready: proceed to chain suggestion (`gwt-plan-spec`).

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
- Planning-ready requires covering the applicable categories from the standard
  question checklist.
- `Question Ledger` records the clarification questions that resolved those
  categories.
- `Depth Gate` is `complete` or `deferred(<reason>)`.

## Output format

```text
## Clarification Report: SPEC-<id>

Resolved: <N> markers
Remaining blockers: <M>
Planning-ready: YES | NO (reason)
Next: gwt-plan-spec | ask follow-up clarification
```
