## Findings

- Current `issue_spec.rs` still documents the Issue body as canonical.
- Artifact comment parsing already exists but only models contract/checklist kinds.
- Viewer code should not absorb storage complexity; storage/API should normalize data before it reaches the UI.
- Migration must handle both repo-local legacy specs and existing GitHub issue-body bundles.
