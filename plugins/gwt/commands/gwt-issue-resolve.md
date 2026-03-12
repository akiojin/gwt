---
description: >-
  Resolve a GitHub Issue end-to-end. Analyze the issue, decide whether it
  should be fixed directly, integrated into an existing gwt-spec issue, or
  promoted to a new spec issue, and continue toward resolution.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GitHub Issue Resolve Command

Use this command as the main entrypoint for GitHub Issues. It should not stop at classification. It should decide the execution path and move the work forward.

## Usage

```text
/gwt:gwt-issue-resolve [issue-number|issue-url|optional context]
```

## Steps

1. Load `skills/gwt-issue-resolve/SKILL.md` and follow the workflow.
2. Run the inspection script to gather issue data and extract context.
3. If the issue is already a spec issue, switch to `gwt-spec-ops`.
4. Otherwise decide the execution path:
   - direct fix
   - existing SPEC
   - new SPEC
5. For SPEC-needed paths, use `gwt-project-index` Issue search before choosing or creating the destination.
6. Continue toward implementation or SPEC execution instead of stopping at triage.

## Proactive Trigger Examples

<example>
Context: User mentions a bug issue and wants it fixed
user: "#42 のバグを直して"
assistant: "gwt-issue-resolve で Issue #42 を解析し、直接修正か SPEC 経由かを判断して進めます。"
</example>

<example>
Context: User provides a feature request issue
user: "https://github.com/org/repo/issues/123 を進めて"
assistant: "gwt-issue-resolve で Issue #123 を解析し、既存 SPEC への統合か新規 SPEC 化を判断して進めます。"
</example>

## Examples

```text
/gwt:gwt-issue-resolve 42
```

```text
/gwt:gwt-issue-resolve https://github.com/org/repo/issues/123
```
