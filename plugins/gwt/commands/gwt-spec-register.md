---
description: Create a new Issue-first SPEC container when no existing canonical SPEC fits, seed `spec.md`, then continue into SPEC orchestration unless register-only was requested.
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT SPEC Register Command

Use this command as a supplementary entrypoint when the user explicitly wants to register a new SPEC, or when `gwt-issue-register` / `gwt-issue-resolve` determine that a new SPEC is required after preflight search.

## Usage

```text
/gwt:gwt-spec-register [issue-number|context]
```

## Steps

1. Load `skills/gwt-spec-register/SKILL.md` and follow the workflow.
2. Use `gwt-issue-search` before creating a new SPEC.
3. If an existing canonical SPEC fits, continue with `gwt-spec-ops` instead of creating a duplicate.
4. Otherwise create a new `gwt-spec` Issue with the artifact-index body and seed `doc:spec.md`.
5. If `gh issue create` or `gh issue edit` is rate-limited, retry through the REST issue endpoints.
6. Return the created issue number to `gwt-spec-ops` unless the user explicitly asked to stop after registration.

## Examples

```text
/gwt:gwt-spec-register #123 から新しい SPEC を起こして
```

```text
/gwt:gwt-spec-register 新機能用の新規 SPEC を登録して
```
