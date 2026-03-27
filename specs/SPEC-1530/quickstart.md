1. List all PRs for the target head branch.
2. If any PR has `mergedAt == null`, mark branch safety as warning.
3. Use latest PR selection only for UI display.
4. Run post-merge commit checks only when all PRs are merged.
