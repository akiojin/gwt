# Research

## Findings

- #1579 is the canonical embedded workflow spec and already owns `gwt-spec-ops` / `gwt-spec-implement` ownership semantics.
- #1296 is superseded and closed, so it is not the right owner for this gap.
- #1327 owns storage/API artifact behavior, not workflow completion semantics.
- #1654 demonstrates that the current workflow can falsely converge on `tasks.md complete` while the implementation still diverges from `doc:spec.md`.
- #1579 itself contains stale checklist debt (`checklist:tdd.md`), so workflow-owned checklist quality must be part of the fix.
