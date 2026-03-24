## Detail View Data Model

- `SpecIssueArtifactKind`
  - `Doc`
  - `Contract`
  - `Checklist`
- `doc:*` artifacts
  - `doc:spec.md`
  - `doc:plan.md`
  - `doc:tasks.md`
  - `doc:research.md`
  - `doc:data-model.md`
  - `doc:quickstart.md`
- `SpecIssueDetail.sections`
  - remains the frontend-facing aggregate shape
  - values are reconstructed from artifact comments first, body fallback second
