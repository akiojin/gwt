## Findings

- `issue_spec.rs` still documents the Issue body as the canonical spec bundle.
- `IssueSpecPanel.svelte` renders `detail.sections.*` directly and does not know how to resolve artifact comments by itself.
- `list_spec_issue_artifact_comments_cmd` and the underlying core API already exist, but are oriented around contract/checklist artifacts today.
- `#1643` is the canonical Git-side search spec; `#1354` is the Issue tab detail spec.

## Implication

The least disruptive design is to keep the frontend detail contract stable and move artifact-first reconstruction into `issue_spec.rs`.
