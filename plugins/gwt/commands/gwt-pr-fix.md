---
description: >-
  Inspect GitHub PR for CI failures, merge conflicts, update-branch requirements,
  reviewer comments, change requests, and unresolved review threads.
  Autonomously fix high-confidence blockers and ask the user only for ambiguous
  conflicts or design decisions.
  Use when: (1) user explicitly asks to fix CI/PR issues,
  (2) after creating or pushing to a PR and CI checks fail or reviews are requested,
  (3) user says 'CIを直して/fix ci/fix pr/マージできない'.
  PROACTIVE: After any PR creation or push, if CI failures or merge blockers
  are detected, automatically invoke this skill without waiting for user request.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GitHub CI Fix Command

Use this command to diagnose and fix CI failures for a PR.

## Usage

```
/gwt:gwt-pr-fix [pr-number|pr-url|optional context]
```

## Steps

1. Load `skills/gwt-pr-fix/SKILL.md` and follow the workflow.
2. Run the inspection script to gather CI results.
3. If the PR is behind the base branch, merge `origin/<base>` into the current branch and push when the merge is clean.
4. If that merge conflicts and the correct resolution is not obvious, ask the user before continuing.
5. Apply high-confidence fixes immediately; ask the user only when a blocker is ambiguous.

## Proactive Trigger Examples

<example>
Context: LLM just created a PR using gwt-pr, and CI checks may fail
user: "PRを作成して"
assistant: (gwt-pr でPR作成完了後) "PR #123 を作成しました。CI チェックの状態を確認します。"
<commentary>
PR作成直後、自動的に gwt-pr-fix で検査する。
</commentary>
</example>

<example>
Context: User pushed commits and mentions the PR isn't passing
user: "pushしたけどCIが通らない"
assistant: "gwt-pr-fix で PR の状態を診断します。"
</example>

<example>
Context: User mentions PR can't be merged
user: "PRがマージできない"
assistant: "gwt-pr-fix で blocking items を診断します。"
</example>

## Examples

```
/gwt:gwt-pr-fix 123
```

```
/gwt:gwt-pr-fix https://github.com/org/repo/pull/123
```
