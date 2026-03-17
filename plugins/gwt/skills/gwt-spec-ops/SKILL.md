---
name: gwt-spec-ops
description: GitHub Issue-first SPEC execution. Use an existing or newly created `gwt-spec` issue to stabilize the spec bundle, maintain plan/tasks/TDD artifacts, and drive implementation progress.
---

# gwt Issue SPEC Ops

GitHub Issues are the single source of truth for specs. Manage every spec as an issue labeled `gwt-spec`.

`gwt-spec-ops` starts after the target SPEC issue has already been identified.

- If the user starts from a plain Issue, use `gwt-issue-resolve` first.
- If the user explicitly needs to create a brand-new SPEC and no canonical SPEC exists yet, use `gwt-spec-register`.
- If the user already has a `gwt-spec` issue number, or the target SPEC destination is already known, continue with this skill.

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

### Issue body sections

The issue body must follow this section structure:

```markdown
<!-- GWT_SPEC_ID:#{number} -->

## Spec

(background, user scenarios, requirements, success criteria)

## Plan

(implementation plan)

## Tasks

(task list)

## TDD

(test design)

## Research

(research notes)

## Data Model

(data model)

## Quickstart

(minimum setup or usage steps)

## Contracts

Artifact files are managed in issue comments with `contract:<name>` entries.

## Checklists

Artifact files are managed in issue comments with `checklist:<name>` entries.

## Acceptance Checklist

- [ ] (acceptance checklist item)
```

### Artifact comments

Manage contracts and checklists as issue comments. Add a marker on the first line:

```markdown
<!-- GWT_SPEC_ARTIFACT:contract:openapi.md -->
contract:openapi.md

(content)
```

## Operations (gh CLI)

### Create new spec issue

```bash
gh issue create --label gwt-spec --title "feat: ..." --body "$(cat <<'EOF'
<!-- GWT_SPEC_ID:#NEW -->

## Spec

_TODO_

## Plan

_TODO_

## Tasks

_TODO_

## TDD

_TODO_

## Research

_TODO_

## Data Model

_TODO_

## Quickstart

_TODO_

## Contracts

Artifact files are managed in issue comments with `contract:<name>` entries.

## Checklists

Artifact files are managed in issue comments with `checklist:<name>` entries.

## Acceptance Checklist

- [ ] Add acceptance checklist
EOF
)"
```

After creation, update the `GWT_SPEC_ID` marker with the issue number:

```bash
gh issue edit {number} --body "$(updated body with <!-- GWT_SPEC_ID:#{number} -->)"
```

### Read spec issue

```bash
gh issue view {number} --json body,title,labels
```

### Update section

```bash
gh issue edit {number} --body "$(updated body)"
```

### Add artifact comment

```bash
gh issue comment {number} --body "$(cat <<'EOF'
<!-- GWT_SPEC_ARTIFACT:contract:openapi.md -->
contract:openapi.md

(content)
EOF
)"
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

1. Update the `## Spec` section only as much as needed to unblock planning and implementation.
2. **Required elements**:
   - **Background**: why this feature or fix is needed
   - **User scenarios**: concrete flows and expected outcomes, with priority P0/P1/P2
   - **Functional requirements**: numbered as `FR-001`
   - **Non-functional requirements**: numbered as `NFR-001` (performance, security, and so on)
   - **Success criteria**: numbered as `SC-001`, with measurable completion conditions
3. Fill missing details from the source Issue, existing comments, or current implementation context before asking the user.
4. Mark unresolved blockers with `[Needs Clarification]` only when they truly block execution.
5. Explicitly document edge cases and error handling that affect implementation or testing.
6. When integrating new work into an existing SPEC, explain the integration choice and reference the related issue numbers.

### 2. Clarify blocking ambiguity

When `## Spec` still contains ambiguous points:

1. Ask at most **5 questions**, ordered by impact.
2. Focus questions on:
   - unclear scope boundaries
   - acceptance criteria that cannot be tested
   - concrete thresholds for non-functional requirements
   - dependencies on other features
3. Replace `[Needs Clarification]` markers with the resolved answers.
4. Reflect both the questions and the answers back into the Spec section.

### 3. Plan (write the implementation plan)

Write the implementation plan in the `## Plan` section:

1. **Technical context**: list the affected files and modules
2. **Implementation approach**: describe the selected architecture and why it was chosen
3. **Phasing**: break the work into staged implementation steps

Generate supporting sections as needed:

- `## Research`: research results such as library selection or API findings
- `## Data Model`: schema and type design
- `## Quickstart`: minimum steps required to run or validate the feature
- `## Contracts`: API contracts managed as artifact comments

### 4. Tasks (generate the task list)

Write the task list in `## Tasks`:

1. **Phase structure**: Setup -> Foundation -> User Stories -> Finalization
2. **Task format**: `- [ ] T001 [Phase] [US1] description`
   - `T001`: sequential ID
   - `[Phase]`: `[S]`etup, `[F]`oundation, `[U]`ser story, `[FIN]`alization
   - `[USn]`: related user scenario number
3. **Dependencies**: document task dependencies explicitly, including `blocked-by` relationships
4. **Completion**: change the checkbox to `[x]` when done

### 5. Analyze (check consistency)

Validate coverage across Spec -> Plan -> Tasks:

1. Confirm that every FR and NFR is covered by Tasks
2. Confirm that every user scenario is mapped to tasks
3. Confirm that there are no circular dependencies
4. Confirm that the work remains testable
5. Propose corrections if you find any gaps or inconsistencies

### 6. Implement (execute tasks)

Task execution procedure:

1. Select the highest-priority unchecked task from `## Tasks`
2. **TDD**: write tests first -> confirm RED -> implement -> confirm GREEN
3. Run independent tasks in parallel when practical
4. Update completed task checkboxes to `[x]`
5. Update the issue body to reflect progress
6. When execution started from a plain Issue, keep the originating Issue linked from the SPEC and reflect status back to the original Issue or PR thread

### 7. Tasks to child issues

When a large task must be split into child issues:

1. Create child issues in dependency order
2. Add links from the parent issue to the child issues in `## Tasks`
3. Add the `gwt-spec` label to child issues that also carry spec content

### 8. Quality checklists

Generate quality checklists for:

- **requirements**: completeness and consistency of requirements
- **security**: security considerations such as OWASP Top 10 coverage
- **ux**: usability and accessibility
- **api**: consistency of API design
- **testing**: completeness of the testing strategy

Add checklists to the issue as artifact comments:

```bash
gh issue comment {number} --body "$(cat <<'EOF'
<!-- GWT_SPEC_ARTIFACT:checklist:requirements.md -->
checklist:requirements.md

- [ ] CHK001 All FR covered by tests
- [ ] CHK002 All NFR have measurable thresholds
...
EOF
)"
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
