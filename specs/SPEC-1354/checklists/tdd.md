## Rust RED tests

- `get_spec_issue_detail` returns sections from `doc:*` artifacts when the Issue body is index-only.
- `get_spec_issue_detail` falls back to legacy body sections when `doc:*` artifacts are absent.
- Mixed mode (`doc:*` + body sections) prefers `doc:*`.

## Frontend RED tests

- `IssueSpecPanel` renders reconstructed sections from an index-only spec issue.
- `IssueListPanel` routes `spec`-labeled issues into `IssueSpecPanel` without relying on body sections.
- Legacy body-canonical spec issues still render correctly.
