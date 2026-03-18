---
description: >-
  Register new GitHub work from a request. Search existing issues and specs
  first, stop on duplicates, then either create a plain issue or switch to
  gwt-spec-register for a new spec.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Issue Register Command

Use this command as the main entrypoint for new work registration.

## Usage

```text
/gwt:gwt-issue-register [request|context]
```

## Steps

1. Load `skills/gwt-issue-register/SKILL.md` and follow the workflow.
2. Normalize the request and classify the work type.
3. Run `gwt-issue-search` first with at least 2 semantic queries.
4. If a clear existing Issue or `gwt-spec` already owns the request, stop new creation and switch to the existing workflow.
5. If the request needs new specification work, switch to `gwt-spec-register`, then continue through `gwt-spec-clarify`, `gwt-spec-plan`, `gwt-spec-tasks`, and `gwt-spec-analyze`.
6. Otherwise create a plain GitHub Issue with the standard section structure.

## Proactive Trigger Examples

<example>
Context: User wants to register a new bug report
user: "この不具合を Issue 化して"
assistant: "gwt-issue-register で既存 Issue / SPEC の重複を確認し、重複がなければ通常 Issue か新規 SPEC かを判断して登録します。"
</example>

<example>
Context: User wants to formalize a new feature request
user: "この要望を起票して"
assistant: "gwt-issue-register で既存の Issue / SPEC を検索し、既存の行き先がなければ通常 Issue か新規 SPEC かを決めて登録します。"
</example>

## Examples

```text
/gwt:gwt-issue-register terminal summary pane bug report
```

```text
/gwt:gwt-issue-register settings redesign request from support note
```
