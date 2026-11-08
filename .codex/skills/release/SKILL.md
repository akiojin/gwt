---
name: release
description: Use when the user requests a new gwt version release or Release PR.
---

## When to use

Prepare an explicitly requested gwt release from a frozen develop snapshot to main. This is a repository-local workflow.

## Completion

Obtain approval for the version and changelog before dispatch. Prepare Release owns snapshot creation; never create or advance it locally. Keep the release bypass scoped and disarm on completion or abort. Verify the merged release workflow and published non-draft Release with all platform assets; PR creation alone is incomplete.

## References

- For approval, dispatch, recovery, and publication checks: [release workflow](references/workflow.md).
- For retained boundaries and optional guidance: [router audit](references/router-audit.md).
