---
name: gwt-execute
description: Use when executing a GitHub Issue, approved SPEC, or approved standalone task.
---

## When to use

Execute one scoped work item through implementation, verification, and delivery. Detect direct, design-gated, or standalone mode from the owner and its artifacts.

## Completion

Keep the same owner through TDD, verify.plan → verify.run, Ready PR, and CI auto-merge. Confirm delivery against the configured base. Preserve execution authority and recovery audits; use No Action only for a proven delivered owner with zero source surface.

With `execution.status` reporting `launch_route: autonomous`, resolve reversible,
in-scope choices using the smallest safe default. Do not stop to wait for a PM reply
or say "ready for the next instruction" after posting a decision. Record
`PM default ruling (override available): <choice and rationale>` on the Board and
continue. After receiving a Board ruling, resume implementation, verification,
PR, or settlement without another `pm.message.send`. Stop for a decision only at
irreversible decisions or `needs_human` boundaries. Existing permission and
delivery gates remain in force; temporary admission or CI waits stay active.

## References

- When establishing scope, stop conditions, and gwtd resolution: [ownership](references/ownership.md).
- Before lifecycle changes, adoption, repair, or recovery: [lifecycle recovery](references/lifecycle-recovery.md).
- When choosing and executing the owner mode: [modes](references/modes.md).
- Before verification, acceptance updates, or a PR handoff: [verification delivery](references/verification-delivery.md).
- For host admission, retries, and compatibility aliases: [admission](references/admission.md).
- For the constraint review: [router audit](references/router-audit.md).
