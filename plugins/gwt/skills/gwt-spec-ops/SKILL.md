---
name: gwt-spec-ops
description: GitHub Issue-first SPEC orchestration. Use an existing or newly created `gwt-spec` issue to stabilize `spec.md`, `plan.md`, `tasks.md`, analysis gates, and then continue into implementation without stopping at normal handoff boundaries.
---

# gwt Issue SPEC Ops

GitHub Issues are the single source of truth for specs. Manage every spec as an issue labeled `gwt-spec`.

`gwt-spec-ops` starts after the target SPEC issue has already been identified.

- If the user starts from a plain Issue, use `gwt-issue-resolve` first.
- If the user explicitly needs to create a brand-new SPEC and no canonical SPEC exists yet, use `gwt-spec-register`.
- If the user already has a `gwt-spec` issue number, or the target SPEC destination is already known, continue with this skill.

`gwt-spec-ops` is the workflow owner. It may call focused subskills, but it should keep driving the work.

- Missing `spec.md` -> seed it through `gwt-spec-register` and continue
- Unresolved clarification -> run `gwt-spec-clarify`, then continue
- Missing plan artifacts -> run `gwt-spec-plan`, then continue
- Missing tasks -> run `gwt-spec-tasks`, then continue
- Missing consistency gate -> run `gwt-spec-analyze`, then continue
- Ready artifact set -> run `gwt-spec-implement`

## Mandatory preflight: search existing spec first

Before you create a new spec issue or decide where to integrate a change, use
`gwt-issue-search` first.

Required behavior:

1. Ensure `gh auth status` is valid before any `index-issues` call
2. Update the Issues index if needed
3. Run semantic Issue search with queries derived from the current request
4. Prefer an existing canonical integrated spec over a transient point-fix/refactor spec
5. Create a new `gwt-spec` Issue only when no suitable canonical spec exists

Typical cases where this preflight is mandatory:

- "既存 spec に統合して"
- "どの仕様に入れるべきか"
- "Project Index の統合仕様を整理して"
- "関連仕様を探してから仕様を書いて"

If `gwt-issue-search` is unavailable or the Issue index is missing, say so and fall back to the
shortest explicit recovery action. Do not silently skip the search.

## Conventions

### SPEC ID

SPEC ID = the **issue number**. Do not use legacy UUID-style spec identifiers.

### Label

An issue with the `gwt-spec` label is a spec issue.

### Issue body contract

The issue body acts as an artifact index, not as the full spec itself:

```markdown
<!-- GWT_SPEC_ID:#{number} -->

## Artifact Index

- `doc:spec.md`
- `doc:plan.md`
- `doc:tasks.md`
- `doc:research.md`
- `doc:data-model.md`
- `doc:quickstart.md`
- `contract:*`
- `checklist:*`

## Status

- Phase: ...
- Clarification: ...
- Analysis: ...

## Links

- Parent: ...
- Related: ...
- PRs: ...
```

### Artifact comments

Manage spec artifacts as issue comments. Add a marker on the first line:

```markdown
<!-- GWT_SPEC_ARTIFACT:doc:plan.md -->
doc:plan.md

(content)
```

Contracts and checklists continue to use:

```markdown
<!-- GWT_SPEC_ARTIFACT:contract:openapi.yaml -->
contract:openapi.yaml

(content)
```

Use the shared helper to list, read, and upsert artifact comments:

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --issue "<number>" \
  --list
```

The helper uses REST issue-comment endpoints for create/update and should be preferred over direct `gh issue comment` calls when artifact writes are needed.

## Operations (gh CLI)

### Read spec issue

```bash
gh issue view {number} --json body,title,labels
```

### Update section

```bash
gh issue edit {number} --body "$(updated body)"
```

If `gh issue edit` is rate-limited, resolve the repo slug with `gh repo view --json nameWithOwner -q .nameWithOwner` and fall back to:

```bash
gh api "repos/<owner>/<repo>/issues/{number}" --method PATCH --input /tmp/issue-edit.json
```

### Add artifact comment

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --issue "{number}" \
  --upsert \
  --artifact "doc:tasks.md" \
  --body-file /tmp/tasks.md
```

### Sync to project

```bash
gh project item-add {project-number} --owner {owner} --url {issue-url}
gh project item-edit --project-id {project-id} --id {item-id} --field-id {field-id} --single-select-option-id {option-id}
```

### List spec issues

```bash
gh issue list --label gwt-spec --state open --json number,title
gh issue list --label gwt-spec --state all --json number,title
```

## Workflow guide

### 0. Search existing spec destination

Before `Specify` or `Plan`, determine whether an existing spec already owns the scope.

1. Use `gwt-issue-search` (`index-issues` + `search-issues`)
2. Search with at least 2 semantic queries derived from the request
3. Rank candidates in this order:
   - canonical integrated spec
   - active feature/bugfix spec covering the same subsystem
   - temporary refactor spec (historical reference only)
4. If an existing canonical spec is found, update it instead of creating a new one
5. Record the chosen destination issue in `## Research` or `## Spec`

### 1. Stabilize the spec for execution

Execution-oriented spec maintenance procedure:

1. Update `doc:spec.md` only as much as needed to unblock planning and implementation.
2. **Required elements**:
   - **Background**: why this feature or fix is needed
   - **User scenarios**: concrete flows and expected outcomes, with priority P0/P1/P2
   - **Functional requirements**: numbered as `FR-001`
   - **Non-functional requirements**: numbered as `NFR-001` (performance, security, and so on)
   - **Success criteria**: numbered as `SC-001`, with measurable completion conditions
3. Fill missing details from the source Issue, existing comments, or current implementation context before asking the user.
4. Mark unresolved blockers with `[NEEDS CLARIFICATION: ...]` only when they truly block execution.
5. Explicitly document edge cases and error handling that affect implementation or testing.
6. When integrating new work into an existing SPEC, explain the integration choice and reference the related issue numbers.

### 2. Clarify blocking ambiguity

When `doc:spec.md` still contains ambiguous points:

1. Run `gwt-spec-clarify` as a focused substep.
2. Resolve everything that can be inferred safely before asking.
3. Focus questions only on:
   - unclear scope boundaries
   - acceptance criteria that cannot be tested
   - concrete thresholds for non-functional requirements
   - dependencies on other features
4. Replace `[NEEDS CLARIFICATION: ...]` markers with the resolved answers.
5. Reflect both the questions and the answers back into `doc:spec.md`.

### 3. Plan (write the planning artifacts)

Run `gwt-spec-plan` to write `doc:plan.md` and supporting artifacts:

1. `doc:plan.md`
2. `doc:research.md`
3. `doc:data-model.md`
4. `doc:quickstart.md`
5. `contract:*`

`doc:plan.md` must include:

- Summary
- Technical Context
- Constitution Check
- Project Structure
- Complexity Tracking
- Phased Implementation

### 4. Generate tasks

Run `gwt-spec-tasks` to produce `doc:tasks.md`.

### 5. Run analysis gate

Run `gwt-spec-analyze` before implementation starts.

Analysis handling rules:

- `CLEAR`: continue directly into `gwt-spec-implement`
- `AUTO-FIXABLE`: repair the artifact set through clarify/plan/tasks as needed, then rerun analysis
- `NEEDS-DECISION`: stop and ask the user only for the missing decision

### 6. Implement the SPEC

When the artifact set is ready:

1. Run `gwt-spec-implement`.
2. Keep issue progress comments current.
3. Use `gwt-pr` and `gwt-pr-fix` to keep PR work moving without waiting for extra permission on routine branch-sync or CI fixes.
4. Return to artifact maintenance only when execution uncovers a real spec bug or newly required clarification.

### 8. Quality checklists

Generate quality checklists for:

- **requirements**: completeness and consistency of requirements
- **security**: security considerations such as OWASP Top 10 coverage
- **ux**: usability and accessibility
- **api**: consistency of API design
- **testing**: completeness of the testing strategy

Add checklists to the issue as artifact comments through the shared helper:

```bash
python3 "${CLAUDE_PLUGIN_ROOT}/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." \
  --issue "{number}" \
  --upsert \
  --artifact "checklist:requirements.md" \
  --body-file /tmp/requirements.md
```

## Integration with normal issues

### Branch creation

```bash
gh issue develop {number}
```

### PR link

Include `Fixes #{number}` in the commit message or PR body to create an automatic link.

### Project phase transition

Use the Phase field to track lifecycle state:

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

- `gh` must be installed and authenticated.
- Repository must have `gwt-spec` label created.
- Agent CWD must be inside the target repository (enforced by gwt worktree hooks).
- `$GWT_PROJECT_ROOT` environment variable is available for explicit repo resolution.

## Stop Conditions

Only stop the workflow for one of these reasons:

- `gh auth status` or required repo access is unavailable
- an existing-owner search is ambiguous and would risk duplicate work
- a product or scope decision remains and the correct answer is not inferable
- a merge conflict or reviewer request cannot be resolved with high confidence
