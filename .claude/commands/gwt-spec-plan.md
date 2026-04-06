---
description: Plan implementation with SDD methodology from spec.md to tasks.md
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---

# SPEC Planning Command

Translate spec.md into SDD architecture, plan.md, tasks.md, and quality gate. Produces research.md, data-model.md, quickstart.md, and contracts.

## Usage

```text
/gwt:gwt-spec-plan [SPEC-ID]
```

## Steps

1. Load `.claude/skills/gwt-spec-plan/SKILL.md` and follow the workflow.
2. Read the target SPEC's spec.md and generate planning artifacts.
3. Run the analysis gate to verify completeness before implementation.

## Examples

```text
/gwt:gwt-spec-plan SPEC-5
```

```text
/gwt:gwt-spec-plan
```

```text
/gwt:gwt-spec-plan --lightweight
```
