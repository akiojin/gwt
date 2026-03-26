---
name: gwt-spec-implement
description: "Implement an existing SPEC end-to-end from tasks.md. Execute test-first tasks, update progress artifacts, and keep PR work moving until the SPEC is done. Use when user says 'implement this spec', 'start implementation', 'execute the tasks', or when a spec has passed the analysis gate and is ready for coding."
---

# gwt SPEC Implement

Use this skill after a SPEC has a stable `spec.md`, `plan.md`, `tasks.md`, and a `CLEAR`
analysis result.

- Primary caller: `gwt-spec-ops`
- If the user only wants spec maintenance, stay in `gwt-spec-ops`
- If execution exposes a real spec bug, route back through `gwt-spec-ops` to repair the artifact set

## Execution ownership

`gwt-spec-implement` is the implementation owner for Spec Kit style execution.

Once started, keep moving until one of these is true:

- the scoped tasks are complete
- a real product or scope decision blocks the next task
- a merge conflict or reviewer request cannot be resolved with high confidence
- required auth or tooling is unavailable

Routine CI failures, update-branch merges, and test-first edits should be handled autonomously.

## Required inputs

- `spec.md`
- `plan.md`
- `tasks.md`
- latest analysis result of `CLEAR`
- optional supporting artifacts: `research.md`, `data-model.md`, `quickstart.md`, `contracts/*`, `checklists/*`

## Workflow

1. **Read the execution context.**
   - Load the SPEC directory, the three core artifacts, and any supporting contracts/checklists.
   - Identify the next incomplete task slice in phase order.

2. **Execute tasks in dependency order.**
   - Finish Setup before Foundational work.
   - Finish shared Foundational work before user-story-specific work.
   - Prefer a narrow task slice that can be verified in one loop.

3. **Work test-first.**
   - Add or update the narrowest failing test that proves the task.
   - Reuse existing test suites when they already cover the target behavior.
   - Do not skip tests when the task changes observable behavior.

4. **Implement the task slice.**
   - Edit only the files implied by `tasks.md` unless execution reveals a missing dependency.
   - Keep the change aligned with `spec.md` and `plan.md`; do not silently widen scope.

5. **Verify before moving on.**
   - Run the smallest meaningful validation set first, then broader checks as the slice stabilizes.
   - If a failure indicates a spec gap rather than a code bug, return to `gwt-spec-ops`.

6. **Update execution tracking.**
   - Update `tasks.md` with completed work when the task format supports completion markers.
   - Append progress updates to `specs/SPEC-{id}/progress.md` using the `Progress / Done / Next` template.
   - Keep progress entries factual and incremental.
   - Do not mark the SPEC complete yet; completion requires the exit gate below.

7. **Keep PR flow moving.**
   - If there is no active PR for the branch, or prior PRs are already merged, use `gwt-pr`.
   - If the current PR is behind, conflicting, failing CI, or has review blockers, use `gwt-pr-fix`.
   - Let those skills handle routine merge/push/fix loops autonomously.

8. **Repeat until the scoped tasks are done.**
   - Continue task-by-task until the SPEC is complete or a true decision blocker remains.

9. **Run the post-implementation completion gate.**
   - Reconcile the implemented behavior against `spec.md`, `tasks.md`, `checklists/acceptance.md`, `checklists/tdd.md`, latest progress files, and executed verification.
   - If these artifacts disagree, return to `gwt-spec-ops` and repair the artifact set or rollback false completion markers.
   - Only after reconciliation passes may the workflow declare the SPEC complete.

## Stop Conditions

Stop only when:

- the next task depends on a product or scope decision that is not inferable
- a merge conflict or review request is ambiguous enough to risk the wrong behavior
- the required repo/auth/tooling access is unavailable

## Completion gate requirements

Before declaring completion:

- every claimed completed task in `tasks.md` must match the implementation
- `checklists/acceptance.md` must reflect actual accepted behavior
- `checklists/tdd.md` must reflect actual verification evidence
- progress entries in `progress.md` must not claim completion that the artifacts or code do not support
- if any of the above diverge, the next step is `gwt-spec-ops`, not `gwt-pr`

## Exit report

```text
## Implementation Report: SPEC-<id>

Completed tasks: <N>
Updated files: <paths>
Verification:
- <command/result>

Next:
- `gwt-pr`
- `gwt-pr-fix`
- return to `gwt-spec-ops`
- ask user for decision
```
