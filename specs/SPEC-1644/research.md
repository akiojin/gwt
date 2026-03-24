# Research
- Closed `#1543` previously carried the generic git operation layer concept, but active open ownership drifted across shell/project/cache specs.
- Existing `list_branch_inventory`, `list_worktree_branches`, `list_worktrees`, cleanup rules, and display-name fallback already behave like a single local Git backend domain.
- `#1654` consumes this domain as shell input and must not become the backend owner.
- `#1714` is the source of truth for issue linkage and exact cache, but not for general local Git backend behavior.
- `#1643` and `#1649` remain GitHub/PR-only boundaries.
