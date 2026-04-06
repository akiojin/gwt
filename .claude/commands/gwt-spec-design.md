---
description: Drive SPEC design from intake to planning-ready using DDD methodology
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# SPEC Design Command

Drive SPEC design with DDD methodology. Runs preflight search, one-question-at-a-time interview, domain discovery, SPEC registration, and clarification.

## Usage

```text
/gwt:gwt-spec-design [args]
```

## Steps

1. Load `.claude/skills/gwt-spec-design/SKILL.md` and follow the workflow.
2. Run preflight search to check for existing SPECs and Issues before creating new ones.
3. Conduct the design interview and produce a planning-ready SPEC.

## Examples

```text
/gwt:gwt-spec-design この機能を設計したい
```

```text
/gwt:gwt-spec-design --deepen SPEC-5
```

```text
/gwt:gwt-spec-design terminal multiplexing feature
```
