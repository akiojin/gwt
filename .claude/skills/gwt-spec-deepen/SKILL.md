---
name: gwt-spec-deepen
description: "This skill should be used when the user wants to deepen an existing SPEC's specifications or detail its tasks further, says 'deepen this spec', 'dig deeper', 'SPECを深掘りして', 'タスクを詳細化して', 'challenge assumptions', 'explore alternatives', 'もっと詰めたい', or wants an interactive workshop on a SPEC's design and implementation plan."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[spec-id]"
---

# gwt SPEC Deepen

Interactively deepen an existing SPEC's specifications and detail its tasks through a two-phase workshop: analysis report followed by focused deep-dive.

This skill operates on existing SPEC artifacts. It reads spec.md, plan.md, and tasks.md, identifies areas that need deeper exploration, and updates those artifacts with the results.

- If no SPEC exists yet, use `gwt-spec-register` first.
- If only `[NEEDS CLARIFICATION]` markers need resolving, `gwt-spec-clarify` may suffice.
- This skill goes beyond clarify: it challenges assumptions, explores alternatives, discovers hidden requirements, and breaks down coarse tasks.

## Phase 1: Analysis Report

Read all SPEC artifacts and produce a structured deepening report.

### Analyze spec.md for

- **Assumption challenges**: premises taken for granted that could be wrong
- **Missing edge cases**: failure modes, concurrency, boundary conditions not addressed
- **Alternative approaches**: design choices that were not compared
- **Vague requirements**: FRs or NFRs that lack measurable criteria
- **User story gaps**: actors, scenarios, or flows not covered
- **Integration blind spots**: interactions with other SPECs or subsystems not explored

### Analyze tasks.md for

- **Coarse tasks**: tasks that span multiple files or concerns and should be split
- **Missing test tasks**: implementation tasks without corresponding test-first entries
- **Dependency gaps**: tasks that implicitly depend on others but lack explicit ordering
- **Unclear scope**: tasks where the expected output or verification is ambiguous
- **Parallelization opportunities**: sequential tasks that could be marked `[P]`

### Report format

Present the report as a numbered list of deepening points, categorized:

```text
## Deepening Report: SPEC-<id>

### Spec-Level (spec.md)
1. [ASSUMPTION] <what is assumed and why it might be wrong>
2. [EDGE CASE] <scenario not covered>
3. [ALTERNATIVE] <design choice not compared>
4. [VAGUE] <requirement lacking measurable criteria>
...

### Task-Level (tasks.md)
1. [COARSE] T-xxx: <why it should be split and into what>
2. [MISSING TEST] T-xxx: <implementation without test-first>
3. [DEPENDENCY] T-xxx -> T-yyy: <implicit dependency>
...

### Recommended Priority
1. <highest-impact point to address first>
2. ...
3. ...
```

Ask the user which points to deep-dive on. Accept numbers, ranges, or "all".

## Phase 2: Interactive Deep-Dive

For each selected point, conduct a focused exploration:

1. Present the current state (quote the relevant artifact section).
2. Ask one targeted question about the point.
3. Wait for the user's answer.
4. Propose a concrete artifact update based on the answer.
5. Apply the update to spec.md or tasks.md after user confirmation.
6. Move to the next selected point.

### Question quality rules

- Ask questions that change implementation, not cosmetic phrasing.
- Present trade-offs with concrete consequences (performance, complexity, UX).
- Challenge obvious answers: "Have you considered the case where...?"
- For task detailing: propose a specific split and ask if the granularity is right.
- Never answer design questions on the user's behalf.

### Artifact updates

- Spec-level changes: update spec.md (user stories, FRs, edge cases, NFRs).
- Task-level changes: update tasks.md (split tasks, add test entries, mark dependencies).
- If deep-dive reveals a plan-level change, note it for `gwt-spec-plan` follow-up.
- Preserve existing content structure; make surgical edits.

## Operations

### Read SPEC artifacts

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --list
```

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --get --artifact "doc:spec.md"
```

### Update artifacts

```bash
python3 ".claude/skills/gwt-spec-ops/scripts/spec_artifact.py" \
  --repo "." --spec "<id>" --upsert \
  --artifact "doc:spec.md" --body-file /tmp/spec.md
```

## Exit criteria

- All selected deepening points have been addressed with user input.
- Artifact updates have been applied.
- A summary of changes is presented.
- If further deepening is needed, suggest re-running with remaining points.

## Integration with gwt-spec-ops

`gwt-spec-ops` may invoke this skill when the analysis gate detects shallow specifications or coarse tasks. After deepening, control returns to `gwt-spec-ops` for the next workflow step.
