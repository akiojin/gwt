---
description: >-
  Compatibility alias for gwt-issue-resolve.
  Use when legacy instructions refer to gwt-issue-ops.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GitHub Issue Ops Alias

Use the same workflow as `/gwt:gwt-issue-resolve`.

## Usage

```text
/gwt:gwt-issue-ops [issue-number|issue-url|optional context]
```

## Steps

1. Load `skills/gwt-issue-ops/SKILL.md`.
2. That skill must delegate to `gwt-issue-resolve` as the source of truth.
3. Follow the `gwt-issue-resolve` workflow without semantic differences.

Prefer `/gwt:gwt-issue-resolve` in new documentation and examples.
