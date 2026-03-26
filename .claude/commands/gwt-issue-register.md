---
description: >-
  Register new GitHub work from a request. Search existing issues and specs
  first, reuse a clear owner when possible, then either create a plain issue or
  continue into the SPEC workflow.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Issue Register Command

Use this command as the main entrypoint for new work registration.

Hard routing rule: when the user asks to issue-file or register new work without an existing GitHub Issue number or URL, invoke this command instead of creating a GitHub Issue directly with `gh issue create`.

## Usage

```text
/gwt:gwt-issue-register [request|context]
```

## Steps

1. Load `skills/gwt-issue-register/SKILL.md` and follow the workflow.
2. Normalize the request and classify the work type.
3. Run `gwt-issue-search` first with at least 2 semantic queries.
4. If a clear existing Issue or `gwt-spec` already owns the request, continue with that workflow instead of creating a duplicate.
5. If the request needs new specification work, create the SPEC through `gwt-spec-register`, then continue through `gwt-spec-ops`.
6. Otherwise create a plain GitHub Issue with the standard section structure.
7. If `gh issue create` is rate-limited, retry through `POST /repos/<owner>/<repo>/issues` with `gh api`.

## Proactive Trigger Examples

<example>
Context: User wants to register a new bug report
user: "この不具合を Issue 化して"
assistant: "新規登録なので、まず gwt-issue-register で既存 Issue / SPEC の重複を確認し、直接 `gh issue create` はせずに通常 Issue か新規 SPEC かを判断して登録します。"
</example>

<example>
Context: User wants to formalize a new feature request
user: "この要望を起票して"
assistant: "新規の起票要求なので、gwt-issue-register で既存の Issue / SPEC を検索し、既存の行き先がなければ通常 Issue か新規 SPEC かを決めます。"
</example>

## Examples

```text
/gwt:gwt-issue-register terminal summary pane bug report
```

```text
/gwt:gwt-issue-register settings redesign request from support note
```
