---
name: gwt-spec-clarify
description: "Clarify an existing SPEC by resolving [NEEDS CLARIFICATION] markers, tightening user stories, and locking acceptance scenarios before planning. Use directly or through gwt-spec-ops. Use when user says 'clarify this spec', 'resolve clarifications', 'tighten the spec', or when spec.md has unresolved markers."
---

# gwt SPEC Clarify

Use this skill to turn a draft `spec.md` artifact into a planning-ready specification.

`gwt-spec-clarify` is a focused clarification step inside the wider SPEC workflow.

- If the target SPEC does not exist yet, use `gwt-spec-register` first.
- If no important clarification remains, return a planning-ready result to `gwt-spec-ops`.
- Do not generate `plan.md` or `tasks.md` here.

## Artifact contract

This skill works on the `spec.md` file in the local SPEC directory:

```text
specs/SPEC-{id}/spec.md
```

`spec.md` must contain these sections:

- Background
- User Stories
- Acceptance Scenarios
- Edge Cases
- Functional Requirements
- Non-Functional Requirements
- Success Criteria

Unknown decisions must be written as `[NEEDS CLARIFICATION: question]`.

## Workflow

1. **Resolve the target SPEC.**
   - Accept a SPEC ID or a request that already has a known destination.
   - If the destination is unknown, use `gwt-issue-search` first.

2. **Read the current `spec.md` artifact.**
   - If it does not exist, seed it through `gwt-spec-register`, then continue.

3. **Find clarification gaps.**
   - Prioritize `[NEEDS CLARIFICATION]` markers.
   - Also look for missing user stories, weak acceptance scenarios, vague requirements, and
     unstated edge cases.

4. **Resolve what is knowable, then present remaining questions to the user.**
   - Fill obvious gaps from the source Issue, existing comments, current implementation, and surrounding artifacts.
   - Identify high-impact questions that change scope, behavior, or testability.
   - Ask at most 5 questions, ordered by implementation impact.
   - **Present questions to the user and STOP. Do not proceed to Step 5 until all questions are answered.**
   - **The agent MUST NOT answer clarification questions on behalf of the user.** If a question requires a product/design decision, the user must answer it.
   - Present each question with numbered options and trade-off explanations where applicable.
   - Use the standard question checklist (see below) to ensure no critical questions are missed.

5. **Update the `spec.md` artifact with user-confirmed decisions.**
   - Replace resolved markers with the user's actual answers, not the agent's assumptions.
   - Record the question asked and the user's answer in a `## Clarification Log` section for traceability.
   - Keep unresolved blockers explicit.
   - Tighten user stories and acceptance scenarios while preserving existing intent.

6. **Decide the next step.**
   - If a real product or scope decision remains, stop with a blocker summary.
   - If the remaining gaps are implementation-local and can be decided safely, resolve them here.
   - If the spec is planning-ready, return control to `gwt-spec-ops` or proceed to `gwt-spec-plan`.

## Standard question checklist

Use the following checklists to identify questions that MUST be asked during clarification. Skip questions only when the answer is explicitly stated in the source Issue or existing artifacts.

### All SPECs

- [ ] v1 scope boundary: what is included, what is explicitly excluded?
- [ ] Integration with existing features: how does this connect to existing functionality?
- [ ] Performance / memory requirements: are there specific thresholds?
- [ ] Error handling policy: what happens on failure?

### New UI component / tile

- [ ] Data storage location and persistence method
- [ ] Default size / layout constraints
- [ ] Relationship with other components / tiles (edge, linked behavior, independent)
- [ ] Library / dependency selection (present trade-offs when multiple candidates exist)
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

The spec may move to planning only when all of the following are true:

- No critical `[NEEDS CLARIFICATION]` markers remain
- All clarification questions have been answered by the user (not resolved by agent assumption)
- Each user story has at least one acceptance scenario
- Edge cases are explicit where implementation could diverge
- Functional and success criteria are testable

## Recommended output

```text
## Clarification Report: SPEC-<id>

Resolved: <N>
Remaining blockers: <M>
Next: `gwt-spec-ops` -> `gwt-spec-plan` | ask follow-up clarification
```

## Operations

### Read current `spec.md`

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --get \
  --artifact "doc:spec.md"
```

### Update `spec.md`

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```
