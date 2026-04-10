---
description: Explore ideas, investigate concerns, and analyze dependencies before committing to implementation
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# SPEC Brainstorm Command

A thinking partner for exploration and judgment. Investigates code, analyzes dependencies, and discusses with the user before deciding on an action.

## Usage

```text
/gwt:gwt-spec-brainstorm [topic or concern]
```

## Steps

1. Load `.claude/skills/gwt-spec-brainstorm/SKILL.md` and follow the workflow.
2. Search existing SPECs and Issues for context.
3. Investigate code, map dependencies, then present findings.
4. Discuss one question at a time until a decision is reached.

## Examples

```text
/gwt:gwt-spec-brainstorm この設計どう思う？
```

```text
/gwt:gwt-spec-brainstorm Hook Registration Table の内容で十分か？
```

```text
/gwt:gwt-spec-brainstorm settings.local.json の生成ロジックに問題がないか調べて
```

```text
/gwt:gwt-spec-brainstorm 壁打ち: マウスクリック対応の依存関係を整理したい
```
