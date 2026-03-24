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
