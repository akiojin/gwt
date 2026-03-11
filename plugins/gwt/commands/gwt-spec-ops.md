---
description: Execute and maintain Issue-first specs (gwt-spec) using the gwt-spec-ops skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Issue Spec Ops Command

Use this command after the target `gwt-spec` issue is already known. It maintains the SPEC bundle and drives plan/tasks/TDD/implementation progress.

## Usage

```text
/gwt:gwt-spec-ops [issue-number|context]
```

## Steps

1. Load `skills/gwt-spec-ops/SKILL.md` and follow the workflow.
2. If the user starts from a plain Issue and no SPEC issue is known yet, switch to `gwt-issue-resolve` first.
3. Ensure `gh auth status` is valid before any issue operation.
4. Create or update the Spec/Plan/Tasks/TDD sections on the target `gwt-spec` issue.
5. Keep SPEC ID as the GitHub issue number and preserve section structure.
6. Report what was changed, what implementation step is next, and what remains unresolved.

## Examples

```text
/gwt:gwt-spec-ops 1288
```

```text
/gwt:gwt-spec-ops SPEC #1288 を進めて
```

```text
/gwt:gwt-spec-ops Project Indexの統合specに今回の修正を取り込んで
```
