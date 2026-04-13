description: Compatibility alias for the visible build entrypoint
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Build Command (Compatibility Alias)
==================================

Legacy command alias. Prefer `/gwt:gwt-build-spec` for visible implementation work and `/gwt:gwt-fix-issue` when the request starts from an existing GitHub Issue.

Usage
-----

```text
/gwt:gwt-spec-build [SPEC-ID or task description]
```

Steps
-----

1. Load `.claude/skills/gwt-build-spec/SKILL.md` and follow the visible workflow.
2. Prefer SPEC mode when a SPEC exists.
3. Use standalone mode only when the task was explicitly approved without a SPEC.

Examples
--------

```text
/gwt:gwt-spec-build SPEC-5
```

```text
/gwt:gwt-spec-build
```

```text
/gwt:gwt-spec-build add clipboard support to the editor widget
```
