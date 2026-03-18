---
name: gwt-spec-clarify
description: Clarify an existing `gwt-spec` by resolving `[NEEDS CLARIFICATION]` markers, tightening user stories, and locking acceptance scenarios before planning. Use after `gwt-spec-register` and before `gwt-spec-plan`.
---

# gwt SPEC Clarify

Use this skill to turn a draft `spec.md` artifact into a planning-ready specification.

`gwt-spec-clarify` is a clarification step, not an implementation step.

- If the target SPEC does not exist yet, use `gwt-spec-register` first.
- If no important clarification remains, hand off to `gwt-spec-plan`.
- Do not generate `plan.md` or `tasks.md` here.

## Artifact contract

This skill works on the `spec.md` issue comment artifact:

```markdown
<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->
doc:spec.md
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
   - Accept a `gwt-spec` issue number or a request that already has a known destination.
   - If the destination is unknown, use `gwt-issue-search` first.

2. **Read the current `spec.md` artifact.**
   - If it does not exist, stop and hand off to `gwt-spec-register`.

3. **Find clarification gaps.**
   - Prioritize `[NEEDS CLARIFICATION]` markers.
   - Also look for missing user stories, weak acceptance scenarios, vague requirements, and
     unstated edge cases.

4. **Ask only high-impact questions.**
   - Ask at most 5 questions, ordered by implementation impact.
   - Focus on scope boundaries, behavioral choices, acceptance criteria, and error handling.

5. **Update the `spec.md` artifact.**
   - Replace resolved markers with concrete decisions.
   - Keep unresolved blockers explicit.
   - Tighten user stories and acceptance scenarios while preserving existing intent.

6. **Decide the next step.**
   - If critical clarification remains, stop with a blocker summary.
   - If the spec is planning-ready, hand off to `gwt-spec-plan`.

## Planning-ready exit criteria

The spec may move to planning only when all of the following are true:

- No critical `[NEEDS CLARIFICATION]` markers remain
- Each user story has at least one acceptance scenario
- Edge cases are explicit where implementation could diverge
- Functional and success criteria are testable

## Recommended output

```text
## Clarification Report: #<number>

Resolved: <N>
Remaining blockers: <M>
Next: `gwt-spec-plan` | ask follow-up clarification
```
