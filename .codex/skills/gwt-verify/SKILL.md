---
name: gwt-verify
description: Use when changed code needs verification before completion or a PR.
---

## When to use

Classify changed surfaces and verify the scoped acceptance criteria with the project’s actual runners. Choose full or approved CI-backed pre-PR mode from repository policy.

## Completion

Register verify.plan and execute the complete matrix through verify.run. Report actual results and missing evidence; require fresh PASS before completion/Ready. Autonomous runs use `User Verification Result: n/a (autonomous)` and separate measured agent visual evidence for UI changes.

## References

- When selecting scope, mode, CI correspondence, and launch route: [contract modes](references/contract-modes.md).
- For isolated headed UI verification and dark/light assertions: [agent visual](references/agent-visual.md).
- When deriving and running the verification plan: [execution](references/execution.md).
- When recording commands, acceptance evidence, and results: [evidence](references/evidence.md).
- Only when a manual launch needs user verification: [handoff](references/handoff.md).
- For admission/retry, stop conditions, and completion: [admission completion](references/admission-completion.md).
- For the constraint review: [router audit](references/router-audit.md).
