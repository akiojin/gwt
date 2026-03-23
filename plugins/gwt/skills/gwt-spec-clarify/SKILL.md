---
name: gwt-spec-clarify
description: Clarify an existing `gwt-spec` by resolving `[NEEDS CLARIFICATION]` markers, tightening user stories, and locking acceptance scenarios before planning. Use directly or through `gwt-spec-ops`.
---

# gwt SPEC Clarify

Use this skill to turn a draft `spec.md` artifact into a planning-ready specification.

`gwt-spec-clarify` is a focused clarification step inside the wider SPEC workflow.

- If the target SPEC does not exist yet, use `gwt-spec-register` first.
- If no important clarification remains, return a planning-ready result to `gwt-spec-ops`.
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
   - If it does not exist, seed it through `gwt-spec-register`, then continue.

3. **Find clarification gaps.**
   - Prioritize `[NEEDS CLARIFICATION]` markers.
   - Also look for missing user stories, weak acceptance scenarios, vague requirements, and
     unstated edge cases.

4. **Resolve what is knowable before asking.**
   - Fill obvious gaps from the source Issue, existing comments, current implementation, and surrounding artifacts.
   - Ask only high-impact questions that change scope, behavior, or testability.
   - Ask at most 5 questions, ordered by implementation impact.

5. **Update the `spec.md` artifact.**
   - Replace resolved markers with concrete decisions.
   - Keep unresolved blockers explicit.
   - Tighten user stories and acceptance scenarios while preserving existing intent.

6. **Decide the next step.**
   - If a real product or scope decision remains, stop with a blocker summary.
   - If the remaining gaps are implementation-local and can be decided safely, resolve them here.
   - If the spec is planning-ready, return control to `gwt-spec-ops` or proceed to `gwt-spec-plan`.

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
Next: `gwt-spec-ops` -> `gwt-spec-plan` | ask follow-up clarification
```

## Operations

### Read current `spec.md`

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --issue "<number>" \
  --get \
  --artifact "doc:spec.md"
```

### Update `spec.md`

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --issue "<number>" \
  --upsert \
  --artifact "doc:spec.md" \
  --body-file /tmp/spec.md
```
