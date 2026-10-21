---
name: browser-check
description: Use when browser verification needs an isolated instance of the current gwt checkout.
---

## When to use

Verify edited gwt code in a fresh checkout instance. Use the autonomous or manual verification route selected by the owning workflow.

## Completion

Use only this checkout’s binaries and an isolated home. Preserve managed-hook authority, credential secrecy, exact PID ownership, and cleanup on every exit. Report observed results; human confirmation and agent evidence remain distinct.

## References

- Before choosing a launch target or sharing evidence: [safety](references/safety.md).
- When preparing checkout binaries and isolated state: [isolation](references/isolation.md).
- When resolving hook authority and starting the owned process: [launch](references/launch.md).
- Before trusting startup, hook convergence, or a fresh URL: [readiness](references/readiness.md).
- For manual inspection, failure reporting, and shutdown: [handoff cleanup](references/handoff-cleanup.md).
- For the constraint review: [router audit](references/router-audit.md).
