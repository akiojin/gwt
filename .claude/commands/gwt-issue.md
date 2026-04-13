description: Compatibility alias for the visible issue registration and issue resolution entrypoints
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

Issue Command (Compatibility Alias)
==================================

Legacy command alias. Prefer `/gwt:gwt-register-issue` for new work intake and `/gwt:gwt-fix-issue` for an existing Issue number or URL.

Usage
-----

```text
/gwt:gwt-issue [issue number, URL, or description]
```

Steps
-----

1. Load `.claude/skills/gwt-issue/SKILL.md` and follow the workflow.
2. If an Issue number or URL is provided, prefer `.claude/skills/gwt-fix-issue/SKILL.md`.
3. If a description is provided, prefer `.claude/skills/gwt-register-issue/SKILL.md`.
4. Continue with the selected visible workflow.

Examples
--------

```text
/gwt:gwt-issue バグ報告: ターミナルリサイズ時にクラッシュ
```

```text
/gwt:gwt-issue #123
```

```text
/gwt:gwt-issue https://github.com/akiojin/gwt/issues/123
```
