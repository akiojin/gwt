# Deepening Reference

Detailed logic for the deepening phase of gwt-discussion.

## Purpose

Challenge assumptions, explore alternatives, discover hidden requirements, and
break down coarse tasks through a two-phase interactive workshop.

## When to use

- Invoke with `gwt-discussion --deepen SPEC-N`.
- Or when the user requests deeper exploration after clarification.
- Goes beyond clarify: it questions premises, not just fills gaps.

## Prerequisites

- SPEC must exist with at least spec.md.
- tasks.md is optional but enables task-level analysis.

## Phase 5.1: Analysis report

### Read all SPEC artifacts

```bash
gwt issue spec <Issue番号>
```

Read spec.md, plan.md, and tasks.md (whichever exist).

### Spec-level analysis (spec.md)

Look for:

| Category | Tag | What to find |
|---|---|---|
| Assumption challenges | [ASSUMPTION] | Premises taken for granted that could be wrong |
| Missing edge cases | [EDGE CASE] | Failure modes, concurrency, boundary conditions |
| Alternative approaches | [ALTERNATIVE] | Design choices that were not compared |
| Vague requirements | [VAGUE] | FRs or NFRs lacking measurable criteria |
| User story gaps | [USER STORY GAP] | Actors, scenarios, or flows not covered |
| Integration blind spots | [INTEGRATION] | Interactions with other SPECs or subsystems |

### Task-level analysis (tasks.md)

Look for:

| Category | Tag | What to find |
|---|---|---|
| Coarse tasks | [COARSE] | Tasks spanning multiple files/concerns, should split |
| Missing test tasks | [MISSING TEST] | Implementation without test-first entry |
| Dependency gaps | [DEPENDENCY] | Implicit dependencies without explicit ordering |
| Unclear scope | [UNCLEAR SCOPE] | Ambiguous expected output or verification |
| Parallelization | [PARALLEL] | Sequential tasks that could be marked [P] |

### Report format

```text
## Deepening Report: SPEC-<id>

### Spec-Level (spec.md)
1. [ASSUMPTION] <what is assumed and why it might be wrong>
2. [EDGE CASE] <scenario not covered>
3. [ALTERNATIVE] <design choice not compared>
4. [VAGUE] <requirement lacking measurable criteria>
5. [USER STORY GAP] <actor or flow not covered>
...

### Task-Level (tasks.md)
1. [COARSE] T-xxx: <why it should be split and into what>
2. [MISSING TEST] T-xxx: <implementation without test-first>
3. [DEPENDENCY] T-xxx -> T-yyy: <implicit dependency>
...

### Recommended Priority
1. <highest-impact point to address first>
2. <second highest>
3. <third highest>
```

After presenting the report, ask the user which points to deep-dive on.
Accept numbers, ranges (e.g., "1-3"), or "all".

## Phase 5.2: Interactive deep-dive

For each selected point:

### Step 1: Present current state

Quote the relevant artifact section verbatim so the user sees exactly what
is being discussed.

### Step 2: Ask one targeted question

Question quality rules:

- **Impact test**: ask questions that change implementation, not cosmetic phrasing.
- **Trade-off framing**: present consequences (performance, complexity, UX).
- **Challenge obvious answers**: "Have you considered the case where...?"
- **Task splits**: for coarse tasks, propose a specific split and ask if the
  granularity is right.
- **Never answer design questions for the user.**

### Step 3: Wait for the answer

Do not proceed until the user responds.

### Step 4: Propose artifact update

Based on the answer, draft a concrete change to spec.md or tasks.md.
Show the diff or new section text.

### Step 5: Apply after confirmation

Update the artifact:

```bash
gwt issue spec <Issue番号> --edit spec -f /tmp/spec.md
```

Or for tasks.md:

```bash
gwt issue spec <Issue番号> --edit tasks -f /tmp/tasks.md
```

### Step 6: Move to next point

Repeat steps 1-5 for each selected deepening point.

## Artifact update rules

- **Spec-level changes**: update spec.md (user stories, FRs, edge cases, NFRs).
- **Task-level changes**: update tasks.md (split tasks, add test entries, mark dependencies).
- **Plan-level changes**: if deep-dive reveals a plan-level change, note it for
  gwt-plan-spec follow-up rather than modifying plan.md directly.
- **Preserve structure**: make surgical edits, do not rewrite entire sections.

## Exit criteria

- All selected deepening points have been addressed with user input.
- Artifact updates have been applied.
- A summary of changes is presented:

```text
## Deepening Summary: SPEC-<id>

Points addressed: <N> / <total selected>
spec.md changes: <list of sections updated>
tasks.md changes: <list of tasks split/added>
Plan-level notes: <any items for gwt-plan-spec follow-up>

Remaining points for future deepening: <list or "none">
```

- If further deepening is needed, suggest re-running with remaining points.
- If the SPEC is now planning-ready, suggest `gwt-plan-spec`.
