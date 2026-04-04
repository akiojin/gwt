# SPEC Ops Workflow (Detailed Steps)

## Step 0: Search existing spec destination

Before `Specify` or `Plan`, determine whether an existing spec already owns the scope.

1. Use `gwt-issue-search` (`index-issues` + `search-issues`)
2. Search local `specs/` via `spec_artifact.py --repo . --list-all`
3. Search with at least 2 semantic queries derived from the request
4. Rank candidates in this order:
   - canonical integrated spec
   - active feature/bugfix spec covering the same subsystem
   - temporary refactor spec (historical reference only)
5. If an existing canonical spec is found, update it instead of creating a new one
6. Record the chosen destination SPEC in `## Research` or `## Spec`

## Step 1: Stabilize the spec for execution

Execution-oriented spec maintenance procedure:

1. Update `spec.md` only as much as needed to unblock planning and implementation.
2. **Required elements**:
   - **Background**: why this feature or fix is needed
   - **User scenarios**: concrete flows and expected outcomes, with priority P0/P1/P2
   - **Functional requirements**: numbered as `FR-001`
   - **Non-functional requirements**: numbered as `NFR-001` (performance, security, and so on)
   - **Success criteria**: numbered as `SC-001`, with measurable completion conditions
3. Fill missing details from the source Issue, existing comments, current implementation context before asking the user.
4. Mark unresolved blockers with `[NEEDS CLARIFICATION: ...]` only when they truly block execution.
5. Explicitly document edge cases and error handling that affect implementation or testing.
6. When integrating new work into an existing SPEC, explain the integration choice and reference the related Issue numbers when they exist.
7. Treat `plan.md`, `tasks.md`, `research.md`, `data-model.md`, `quickstart.md`, `analysis.md`, `contracts/*`, `checklists/*`, and `progress.md` as the local artifact set that downstream viewers and completion gates consume.

## Step 2: Clarify blocking ambiguity

When `spec.md` still contains ambiguous points:

1. Run `gwt-spec-clarify` as a focused substep.
2. Resolve what can be inferred safely from source Issues, existing code, and artifacts.
3. For remaining questions, present them to the user using the standard question checklist in `gwt-spec-clarify`.
4. **STOP and wait for user answers before proceeding to Step 3 (Plan).** Do not assume answers or proceed with agent-generated decisions.
5. Replace `[NEEDS CLARIFICATION: ...]` markers with the user's confirmed answers.
6. Reflect both the questions and the answers back into `spec.md`.

## Phase transition gates

Before advancing to the next workflow step, verify the gate condition:

| Transition | Gate condition |
|---|---|
| Clarify -> Plan | All [NEEDS CLARIFICATION] resolved with user-confirmed answers |
| Plan -> Tasks | plan.md reviewed and consistent with clarified spec.md |
| Tasks -> Analyze | tasks.md covers all user stories and functional requirements |
| Analyze -> Implement | Analysis result is CLEAR or all AUTO-FIXABLE items resolved |

## Step 3: Plan (write the planning artifacts)

Run `gwt-spec-plan` to write `plan.md` and supporting artifacts:

1. `plan.md`
2. `research.md`
3. `data-model.md`
4. `quickstart.md`
5. `contracts/*`

`plan.md` must include:

- Summary
- Technical Context
- Constitution Check
- Project Structure
- Complexity Tracking
- Phased Implementation

## Step 4: Generate tasks

Run `gwt-spec-tasks` to produce `tasks.md`.

## Step 5: Run analysis gate

Run `gwt-spec-analyze` before implementation starts.

Analysis handling rules:

- Persist the analysis report to `analysis.md`
- `CLEAR`: continue directly into `gwt-spec-implement`
- `AUTO-FIXABLE`: repair the artifact set through clarify/plan/tasks as needed, then rerun analysis
- `NEEDS-DECISION`: stop and ask the user only for the missing decision

## Step 6: Implement the SPEC

When the artifact set is ready:

1. Run `gwt-spec-implement`.
2. Keep local progress files current (e.g., `specs/SPEC-{id}/progress.md`) and refresh `analysis.md` whenever artifact repair changes the readiness judgment.
3. Use `gwt-pr` and `gwt-pr-fix` to keep PR work moving without waiting for extra permission on routine branch-sync or CI fixes.
4. After implementation, require a completion-gate reconciliation across `spec.md`, `tasks.md`, `analysis.md`, `checklists/acceptance.md`, `checklists/tdd.md`, progress files, and verification evidence before treating the SPEC as complete.
5. Return to artifact maintenance whenever execution uncovers a real spec bug, false completion markers, malformed checklist artifacts, or newly required clarification.

## Step 8: Quality checklists

Generate quality checklists for:

- **requirements**: completeness and consistency of requirements
- **security**: security considerations such as OWASP Top 10 coverage
- **ux**: usability and accessibility
- **api**: consistency of API design
- **testing**: completeness of the testing strategy

Add checklists to the SPEC directory through the shared helper:

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "checklist:requirements.md" \
  --body-file /tmp/requirements.md
```
