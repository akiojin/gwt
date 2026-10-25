---
name: gwt-manage-pr
description: Use when creating, inspecting, fixing, or delivering a pull request.
---

## When to use

Manage the requested PR action in the current branch. Select create, check, fix, or deliver from the request and launch route.

## Completion

Report the actual PR action, URL, changed scope, verification, and remaining blockers. Before Ready or merge automation, require fresh canonical evidence and the repository’s releaseable-slice gates. Autonomous delivery continues through CI auto-merge; inspect-only work makes no mutation.

## References

- When determining scope, base, source commits, and mode: [routing preflight](references/routing-preflight.md).
- Before any Ready handoff or delivery: [ready gate](references/ready-gate.md).
- When creating or updating a PR: [create](references/create.md).
- For read-only PR status inspection: [check](references/check.md).
- When diagnosing or repairing CI/review/conflict blockers: [fix](references/fix.md).
- When driving an authorized PR to merge and reporting the outcome: [deliver](references/deliver.md).
- For the constraint review: [router audit](references/router-audit.md).
