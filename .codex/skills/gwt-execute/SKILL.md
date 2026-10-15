---
name: gwt-execute
description: Use when executing a GitHub Issue, approved SPEC, or approved standalone task.
---

## When to use

Execute one scoped work item through implementation, verification, and delivery. Detect direct, design-gated, or standalone mode from the owner and its artifacts.

## Completion

Keep the same owner through TDD, verify.plan → verify.run, Ready PR, and CI auto-merge. Confirm delivery against the configured base. Preserve execution authority and recovery audits; use No Action only for a proven delivered owner with zero source surface.

## References

- When establishing scope, stop conditions, and gwtd resolution: [ownership](references/ownership.md).
- Before lifecycle changes, adoption, repair, or recovery: [lifecycle recovery](references/lifecycle-recovery.md).
- When choosing and executing the owner mode: [modes](references/modes.md).
- Before verification, acceptance updates, or a PR handoff: [verification delivery](references/verification-delivery.md).
- For host admission, retries, and compatibility aliases: [admission](references/admission.md).
- For the constraint review: [router audit](references/router-audit.md).
