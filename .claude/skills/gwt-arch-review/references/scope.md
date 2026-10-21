# gwt-arch-review — Codebase Architecture Review

Perform a structured architectural review of any codebase and produce a prioritized improvement report. This skill is read-only — it analyzes and reports but never modifies code.

## When to use

- Weekly or bi-weekly codebase health checks
- Before major feature work to identify structural debt
- After a series of rapid changes to detect architectural drift
- When agent output quality degrades (often signals codebase degradation)

## Core principle

> "If you have a garbage code base, the AI will produce garbage within that code base."
> — aihero.dev

Regular architectural review directly improves agent output quality. This skill closes the feedback loop:

```text
gwt-discussion → gwt-plan-spec → gwt-build-spec → gwt-arch-review
     ↑                                    |
     └────────────────────────────────────┘
```

## Scope control

Before starting, determine the review scope with the caller:

- **`--scope repo`**: review all production source code in the repository
- **`--scope changed --base <ref>`**: review the files changed since the selected base ref (`git diff --name-only <ref>..HEAD`)

If scope is not explicit or inferable, clarify between:

- **Full repository**
- **Changed files since a base ref**

If the caller selects `changed` interactively, prompt for the base ref next.
Non-interactive runs must pass `--base` when using `--scope changed`.

Exclude from analysis: test fixtures, generated code, vendored dependencies, build artifacts.
