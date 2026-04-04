---
name: gwt-spec-ops
description: "This skill should be used when the user wants to drive a SPEC end-to-end, says 'run spec workflow', 'orchestrate this spec', 'stabilize the spec', 'SPECを進めて', 'SPEC-Nを実装まで持っていって', or asks to manage spec.md, plan.md, tasks.md, and analysis gates through implementation. It orchestrates the full SPEC lifecycle from clarification through implementation without stopping at normal handoff boundaries."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[spec-id]"
---

# gwt SPEC Ops

Local SPEC directories (`specs/SPEC-{id}/`) are the single source of truth for specs.

GitHub Issues are optional related records. They are not spec containers and must not be treated
as the canonical source for SPEC detail, planning artifacts, or completion state.

`gwt-spec-ops` starts after the target SPEC has already been identified.

- If the user starts from a plain Issue, use `gwt-issue-resolve` first.
- If the user explicitly needs to create a brand-new SPEC and no canonical SPEC exists yet, use `gwt-spec-register`.
- If the user already has a SPEC ID, or the target SPEC destination is already known, continue with this skill.

`gwt-spec-ops` is the workflow owner. It may call focused subskills, but it should keep driving the work.

- Missing `spec.md` -> seed it through `gwt-spec-register` and continue
- Unresolved clarification -> run `gwt-spec-clarify`, then continue
- Missing plan artifacts -> run `gwt-spec-plan`, then continue
- Missing tasks -> run `gwt-spec-tasks`, then continue
- Missing consistency gate -> run `gwt-spec-analyze`, then continue
- Ready artifact set -> run `gwt-spec-implement`

## Mandatory preflight: search existing spec first

Before you create a new SPEC or decide where to integrate a change, use
`gwt-issue-search` first.

Required behavior:

1. Update the Issues index if needed
2. Search local `specs/` directory via `spec_artifact.py --repo . --list-all`
3. Run semantic search with queries derived from the current request
4. Prefer an existing canonical integrated spec over a transient point-fix/refactor spec
5. Create a new SPEC only when no suitable canonical spec exists

Typical cases where this preflight is mandatory:

- "既存 spec に統合して"
- "どの仕様に入れるべきか"
- "Project Index の統合仕様を整理して"
- "関連仕様を探してから仕様を書いて"

If `gwt-issue-search` is unavailable or the index is missing, say so and fall back to the
shortest explicit recovery action. Do not silently skip the search.

## Conventions

### SPEC ID

SPEC ID = the directory name suffix (e.g., `a1b2c3d4` from `specs/SPEC-a1b2c3d4/`). Do not use legacy UUID-style spec identifiers or GitHub Issue numbers as SPEC IDs.

### SPEC directory structure

Each SPEC is stored as a local directory:

```text
specs/SPEC-{id}/
  metadata.json      # {"id","title","status","phase","created_at","updated_at"}
  spec.md
  plan.md
  tasks.md
  research.md
  data-model.md
  quickstart.md
  analysis.md
  contracts/
  checklists/
```

### `metadata.json`

The `metadata.json` file tracks SPEC status and phase:

```json
{
  "id": "SPEC-{id}",
  "title": "gwt-spec: <description>",
  "status": "open",
  "phase": "Specify",
  "created_at": "2024-01-01T00:00:00Z",
  "updated_at": "2024-01-01T00:00:00Z"
}
```

Use the shared helper to list, read, and upsert artifact files:

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --list
```

## Operations

### Read SPEC metadata

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --get \
  --artifact "metadata"
```

### Update artifact

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "doc:tasks.md" \
  --body-file /tmp/tasks.md
```

### List all SPECs

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --list-all
```

### Close SPEC

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --close
```

### Add quality checklist

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --spec "<id>" \
  --upsert \
  --artifact "checklist:requirements.md" \
  --body-file /tmp/requirements.md
```

## Workflow guide

### 0. Search existing spec destination

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

### 1. Stabilize the spec for execution

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

### 2. Clarify blocking ambiguity

When `spec.md` still contains ambiguous points:

1. Run `gwt-spec-clarify` as a focused substep.
2. Resolve what can be inferred safely from source Issues, existing code, and artifacts.
3. For remaining questions, present them to the user using the standard question checklist in `gwt-spec-clarify`.
4. **STOP and wait for user answers before proceeding to Step 3 (Plan).** Do not assume answers or proceed with agent-generated decisions.
5. Replace `[NEEDS CLARIFICATION: ...]` markers with the user's confirmed answers.
6. Reflect both the questions and the answers back into `spec.md`.

### Phase transition gates

Before advancing to the next workflow step, verify the gate condition:

| Transition | Gate condition |
|---|---|
| Clarify → Plan | All [NEEDS CLARIFICATION] resolved with user-confirmed answers |
| Plan → Tasks | plan.md reviewed and consistent with clarified spec.md |
| Tasks → Analyze | tasks.md covers all user stories and functional requirements |
| Analyze → Implement | Analysis result is CLEAR or all AUTO-FIXABLE items resolved |

### 3. Plan (write the planning artifacts)

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

### 4. Generate tasks

Run `gwt-spec-tasks` to produce `tasks.md`.

### 5. Run analysis gate

Run `gwt-spec-analyze` before implementation starts.

Analysis handling rules:

- Persist the analysis report to `analysis.md`
- `CLEAR`: continue directly into `gwt-spec-implement`
- `AUTO-FIXABLE`: repair the artifact set through clarify/plan/tasks as needed, then rerun analysis
- `NEEDS-DECISION`: stop and ask the user only for the missing decision

### 6. Implement the SPEC

When the artifact set is ready:

1. Run `gwt-spec-implement`.
2. Keep local progress files current (e.g., `specs/SPEC-{id}/progress.md`) and refresh `analysis.md` whenever artifact repair changes the readiness judgment.
3. Use `gwt-pr` and `gwt-pr-fix` to keep PR work moving without waiting for extra permission on routine branch-sync or CI fixes.
4. After implementation, require a completion-gate reconciliation across `spec.md`, `tasks.md`, `analysis.md`, `checklists/acceptance.md`, `checklists/tdd.md`, progress files, and verification evidence before treating the SPEC as complete.
5. Return to artifact maintenance whenever execution uncovers a real spec bug, false completion markers, malformed checklist artifacts, or newly required clarification.

### 8. Quality checklists

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

## Integration with normal issues

### Branch creation

```bash
gh issue develop {issue-number}
```

### PR link

Include `Fixes #{issue-number}` in the commit message or PR body to create an automatic link to the originating GitHub Issue.

### SPEC phase tracking

Use the `phase` field in `metadata.json` to track lifecycle state:

| Phase | Meaning |
|---|---|
| Draft | Spec drafting in progress |
| Ready | Spec complete, waiting for review |
| Planned | Planning completed |
| Ready for Dev | Ready to begin implementation |
| In Progress | Implementation in progress |
| Done | Completed |
| Blocked | Blocked |

## Requirements

- Agent CWD must be inside the target repository (enforced by gwt worktree hooks).
- `$GWT_PROJECT_ROOT` environment variable is available for explicit repo resolution.

## Stop Conditions

Only stop the workflow for one of these reasons:

- required repo access is unavailable
- an existing-owner search is ambiguous and would risk duplicate work
- a product or scope decision remains and the correct answer is not inferable
- a merge conflict or reviewer request cannot be resolved with high confidence
- unanswered clarification questions remain from `gwt-spec-clarify` (must wait for user response before proceeding to plan/tasks)
