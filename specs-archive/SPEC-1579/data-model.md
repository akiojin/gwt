## Canonical Ownership Model

- Workflow/Storage/API/Completion canonical: #1579
- Viewer / detail rendering: `SPEC-1776` for local SPEC viewing, `#1354` for GitHub Issue detail / legacy issue-body compatibility
- Search / discovery: #1643

## Lifecycle Order

1. `gwt-issue-register`
2. `gwt-spec-register`
3. `gwt-spec-clarify`
4. `gwt-spec-plan`
5. `gwt-spec-tasks`
6. `gwt-spec-analyze`
7. `gwt-spec-ops`

## GitHub Transport Capability Matrix

- PR list / create / update / view metadata: REST-first
- Commit status / check-runs: REST-first
- PR comments / issue comments: REST-first
- Reviews / review comments: REST-first
- Unresolved review thread discovery: GraphQL-only for now
- Review thread reply / resolve: GraphQL-only for now

## Canonical Entities

- `SpecIssueDetail`
  - unchanged aggregate shape for frontend consumption
- `SpecIssueArtifactKind`
  - `Doc`
  - `Contract`
  - `Checklist`
- Canonical artifact keys
  - `doc:spec.md`
  - `doc:plan.md`
  - `doc:tasks.md`
  - `doc:research.md`
  - `doc:data-model.md`
  - `doc:quickstart.md`
  - `contract:*`
  - `checklist:*`

## Fallback Rule

1. `doc:*` artifacts
2. legacy body sections
3. empty/default section value

## Completion Gate

### Inputs

- current code state
- executed verification commands
- `doc:spec.md`
- `doc:tasks.md`
- `checklist:tdd.md`
- `checklist:acceptance.md`
- latest progress comments

### Outputs

- `PASS`: completion may be declared
- `BLOCKED`: return to `gwt-spec-ops` and repair artifacts or implementation
