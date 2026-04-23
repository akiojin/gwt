---
description: Register new work through the visible issue intake workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Register Issue Command
======================

Public task entrypoint for registering new work from a description, bug report, or enhancement idea.

Usage
-----

```text
/gwt:gwt-register-issue [description]
```

Steps
-----

1. Load `.claude/skills/gwt-register-issue/SKILL.md` and follow the workflow.
2. Search for duplicates before creating anything.
3. Decide `Spec Status` (`ALIGNED`, `IMPLEMENTATION-GAP`, `SPEC-GAP`, `SPEC-AMBIGUOUS`) before choosing the owner.
4. Create a plain Issue only for narrow `ALIGNED` or `IMPLEMENTATION-GAP` work, and route `SPEC-GAP` / `SPEC-AMBIGUOUS` to SPEC discussion instead.

Examples
--------

```text
/gwt:gwt-register-issue バグ報告: ターミナルリサイズ時にクラッシュ
```

```text
/gwt:gwt-register-issue Add richer issue filtering for the Issues tab
```
