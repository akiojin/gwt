---
description: >-
  Analyze a GitHub Issue, extract error context, search the codebase for
  relevant files, route spec issues to gwt-spec-ops, and propose a concrete
  plan for non-spec issues.
  Use when: (1) user explicitly asks to analyze or work on an issue,
  (2) user provides an issue number or URL and asks for help,
  (3) user says 'Issueを直して/fix issue/analyze issue/investigate #123'.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GitHub Issue Ops Command

Use this command to inspect a GitHub Issue, decide whether it is a spec issue, and either route it to `gwt-spec-ops` or produce a concrete issue analysis/fix plan.

## Usage

```
/gwt:gwt-issue-ops [issue-number|issue-url|optional context]
```

## Steps

1. Load `skills/gwt-issue-ops/SKILL.md` and follow the workflow.
2. Run the inspection script to gather issue data and extract context.
3. If the issue is a spec issue, switch to `gwt-spec-ops` instead of continuing.
4. Otherwise search the codebase for relevant files and definitions.
5. Produce an Issue Analysis Report and propose fixes after user approval.

## Proactive Trigger Examples

<example>
Context: User mentions an issue number and asks for help
user: "#42 のバグを直して"
assistant: "gwt-issue-ops で Issue #42 を分析します。"
<commentary>
Issue 番号が指定されたので gwt-issue-ops で分析を開始する。
</commentary>
</example>

<example>
Context: User provides an issue URL
user: "https://github.com/org/repo/issues/123 を調査して"
assistant: "gwt-issue-ops で Issue #123 を分析し、必要なら spec workflow に切り替えます。"
</example>

<example>
Context: User asks to investigate a bug
user: "この Issue を進めたい"
assistant: "gwt-issue-ops で Issue を分析し、spec issue なら gwt-spec-ops に切り替え、通常 issue なら修正計画を提案します。"
</example>

## Examples

```
/gwt:gwt-issue-ops 42
```

```
/gwt:gwt-issue-ops https://github.com/org/repo/issues/123
```
