---
description: Manage Issue-first specs (gwt-spec) using the gwt-issue-spec-ops skill
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# GWT Issue Spec Ops Command

Use this command to create/update Issue-first SPEC artifacts on GitHub Issues.

## Usage

```text
/gwt:gwt-issue-spec-ops [issue-number|context]
```

## Steps

1. Load `skills/gwt-issue-spec-ops/SKILL.md` and follow the workflow.
2. Before creating/updating a spec, use `gwt-project-index` Issue search to find the canonical existing spec destination.
3. Ensure `gh auth status` is valid before any `index-issues` or issue operation.
4. Create or update the Spec/Plan/Tasks sections on the target `gwt-spec` issue.
5. Keep SPEC ID as the GitHub issue number and preserve section structure.
6. Report what was changed, which existing spec was selected, and what remains unresolved.

## Examples

```text
/gwt:gwt-issue-spec-ops 1288
```

```text
/gwt:gwt-issue-spec-ops 新機能のspecを作成して
```

```text
/gwt:gwt-issue-spec-ops Project Indexの統合specに今回の修正を取り込んで
```
