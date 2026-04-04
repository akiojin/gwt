---
name: gwt-spec-ops
description: "This skill should be used when the user wants to drive a SPEC end-to-end, says 'run spec workflow', 'orchestrate this spec', 'stabilize the spec', 'SPECを進めて', 'SPEC-Nを実装まで持っていって', or asks to manage spec.md, plan.md, tasks.md, and analysis gates through implementation. It orchestrates the full SPEC lifecycle from clarification through implementation without stopping at normal handoff boundaries."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[spec-id]"
---

# gwt SPEC Ops

## Overview

Local SPEC directories (`specs/SPEC-{id}/`) are the single source of truth for specs. GitHub Issues are optional related records, not spec containers.

`gwt-spec-ops` starts after the target SPEC has already been identified. It is the workflow owner that may call focused subskills but keeps driving the work.

### Routing

- If the user starts from a plain Issue, use `gwt-issue-resolve` first.
- If the user needs a brand-new SPEC and no canonical SPEC exists, use `gwt-spec-register`.
- If the user already has a SPEC ID, continue with this skill.

### Subskill dispatch

- Missing `spec.md` -> seed through `gwt-spec-register` and continue
- Unresolved clarification -> run `gwt-spec-clarify`, then continue
- Missing plan artifacts -> run `gwt-spec-plan`, then continue
- Missing tasks -> run `gwt-spec-tasks`, then continue
- Missing consistency gate -> run `gwt-spec-analyze`, then continue
- Ready artifact set -> run `gwt-spec-implement`

## Mandatory preflight: search existing spec first

Before creating a new SPEC or deciding where to integrate a change, use `gwt-issue-search` first.

1. Update the Issues index if needed
2. Search local `specs/` directory via `spec_artifact.py --repo . --list-all`
3. Run semantic search with queries derived from the current request
4. Prefer an existing canonical integrated spec over a transient point-fix/refactor spec
5. Create a new SPEC only when no suitable canonical spec exists

If `gwt-issue-search` is unavailable or the index is missing, say so and fall back to the shortest explicit recovery action. Do not silently skip the search.

## Conventions

### SPEC ID

SPEC ID = the directory name suffix (e.g., `a1b2c3d4` from `specs/SPEC-a1b2c3d4/`). Do not use legacy UUID-style spec identifiers or GitHub Issue numbers as SPEC IDs.

### SPEC directory structure

```text
specs/SPEC-{id}/
  metadata.json      # {"id","title","status","phase","created_at","updated_at"}
  spec.md
  plan.md
  tasks.md
  research.md
  data-model.md
  quickstart.md
  analysis.md
  contracts/
  checklists/
```

## Operations

### Read SPEC metadata

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --get --artifact "metadata"
```

### Update artifact

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --upsert --artifact "doc:tasks.md" --body-file /tmp/tasks.md
```

### List all SPECs

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" --repo "." --list-all
```

### Close SPEC

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --close
```

## Workflow summary

1. Search existing spec destination (mandatory preflight)
2. Stabilize `spec.md` for execution
3. Clarify blocking ambiguity (STOP for user answers before planning)
4. Plan: write `plan.md` and supporting artifacts via `gwt-spec-plan`
5. Generate `tasks.md` via `gwt-spec-tasks`
6. Run analysis gate via `gwt-spec-analyze`
7. Implement via `gwt-spec-implement`
8. Generate quality checklists

Load `references/workflow.md` for detailed step-by-step instructions, spec stabilization requirements, phase transition gates, and quality checklist generation.

## `metadata.json` format

```json
{
  "id": "SPEC-{id}",
  "title": "gwt-spec: <description>",
  "status": "open",
  "phase": "Specify",
  "created_at": "2024-01-01T00:00:00Z",
  "updated_at": "2024-01-01T00:00:00Z"
}
```

### SPEC phase tracking

| Phase | Meaning |
|---|---|
| Draft | Spec drafting in progress |
| Ready | Spec complete, waiting for review |
| Planned | Planning completed |
| Ready for Dev | Ready to begin implementation |
| In Progress | Implementation in progress |
| Done | Completed |
| Blocked | Blocked |

## Integration with normal issues

- Branch creation: `gh issue develop {issue-number}`
- PR link: include `Fixes #{issue-number}` in the commit message or PR body

## Requirements

- Agent CWD must be inside the target repository.
- `$GWT_PROJECT_ROOT` environment variable is available for explicit repo resolution.

## Stop Conditions

Only stop the workflow for one of these reasons:

- required repo access is unavailable
- an existing-owner search is ambiguous and would risk duplicate work
- a product or scope decision remains and the correct answer is not inferable
- a merge conflict or reviewer request cannot be resolved with high confidence
- unanswered clarification questions remain from `gwt-spec-clarify`

## References

- `references/workflow.md`: Detailed workflow steps, spec stabilization, phase gates, quality checklists
