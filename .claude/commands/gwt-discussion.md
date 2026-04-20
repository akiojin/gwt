---
description: Investigate ideas, spec gaps, and implementation concerns through the unified gwt discussion workflow
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# gwt Discussion Command

Unified discussion entrypoint for idea exploration, spec clarification, and
mid-implementation investigation.

## Usage

```text
/gwt:gwt-discussion [topic or concern]
```

## Steps

1. Enter Plan Mode before starting the discussion, then load `.claude/skills/gwt-discussion/SKILL.md` and follow the workflow.
2. Search existing SPECs and Issues for context.
3. Investigate code, map dependencies, then present findings before proposing a path.
4. Discuss one question at a time with selection UI first. In Codex, use `request_user_input` when that UI is available.
5. After each answer, update `Discussion TODO`, `Coverage Checks`, and `Exit Blockers`, then re-rank unresolved high-impact unknowns and ask the next highest-impact question before exiting.
6. Do not leave Plan Mode until `Action Delta` and `Action Bundle` are ready and the depth gate is satisfied or intentionally deferred.
7. If managed hooks surface an unfinished discussion prompt, use `.gwt/discussion.md` as the source of truth and choose `Resume discussion`, `Park proposal`, or `Dismiss for now`.

## Examples

```text
/gwt:gwt-discussion この設計どう思う？
```

```text
/gwt:gwt-discussion Hook Registration Table の内容で十分か？
```

```text
/gwt:gwt-discussion settings.local.json の生成ロジックに問題がないか調べて
```

```text
/gwt:gwt-discussion 実装途中で plan とコードの前提がずれていないか確認したい
```
