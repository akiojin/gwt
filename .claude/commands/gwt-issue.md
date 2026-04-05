---
description: Unified GitHub Issue lifecycle manager for registration and resolution
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# Issue Command

Unified GitHub Issue lifecycle manager. Auto-detects mode: no Issue number means register mode (search first, then create Issue or SPEC); Issue number/URL means resolve mode (analyze, decide direct fix vs SPEC path, continue toward resolution).

## Usage

```text
/gwt:gwt-issue [issue number, URL, or description]
```

## Steps

1. Load `.claude/skills/gwt-issue/SKILL.md` and follow the workflow.
2. If an Issue number or URL is provided, enter resolve mode.
3. If a description is provided, search for duplicates first, then register a new Issue.

## Examples

```text
/gwt:gwt-issue バグ報告: ターミナルリサイズ時にクラッシュ
```

```text
/gwt:gwt-issue #123
```

```text
/gwt:gwt-issue https://github.com/akiojin/gwt/issues/123
```
