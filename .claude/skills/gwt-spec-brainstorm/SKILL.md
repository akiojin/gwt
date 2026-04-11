---
name: gwt-spec-brainstorm
description: "Use when the user has a rough idea, a concern about existing design, wants iterative selection-UI questioning before committing to implementation, or when implementation reveals a gap or inconsistency that needs investigation. Triggers: 'brainstorm', '壁打ち', 'SPECにする前に相談', '質問UIで議論', '選択UIで質問して', 'これって問題じゃない?', 'ここ足りなくない?', 'should this be a new spec?', 'この設計どう思う?', '依存関係を整理して'."
---

# gwt-spec-brainstorm

A thinking partner for exploration and judgment. Stays in investigation
space during discovery — does not produce new specs, plans, or code
while the discussion is still fluid, but may update an existing owner
SPEC once the direction is stable.

## When to use

- A rough idea or request that may or may not become a SPEC
- A concern about an existing SPEC or implementation ("this seems wrong")
- Mid-implementation discovery of a gap, inconsistency, or dependency
  that needs discussion before continuing
- Any open-ended "what if" or "how about" question about design

## Core principles

1. **Do not produce artifacts prematurely.** During investigation and
   discussion, do not write new SPEC artifacts, plans, tasks, or code.
   If the clear owner is an existing SPEC, update that owner only after
   the discussion stabilizes, and do it in one batch.
2. **Investigate before asking.** Read code, grep for patterns, check
   existing SPECs and Issues, run experiments before forming a
   question. Informed questions are high-value; generic questions
   waste the user's time.
3. **Analyze dependencies before discussing changes.** Every proposed
   change has upstream prerequisites, downstream impacts, and
   atomicity boundaries. Map these before presenting options.
4. **One question at a time, selection UI first.** Use a platform
   question tool rather than hard-coding a product-specific API.
   Prefer 2-3 mutually exclusive options and put the recommended
   option first. Fall back to plain text only when no question UI is
   available, or when the decision cannot be represented honestly with
   options.
5. **Keep asking until uncertainty is resolved.** Do not stop after one
   answer if high-impact unknowns remain. Maintain an internal list of
   unresolved questions and continue through it in priority order.
6. **Present findings, not conclusions.** Show what you found (table,
   diff, dependency chain) and let the user drive the decision.
7. **Track the thread.** Maintain an Intake Memo in the conversation.
   Update it as the discussion evolves, along with candidate SPEC deltas
   when an existing owner is identified.
8. **Multiple exits are valid.** Not every discussion becomes a SPEC.

## Platform question tool

Use the platform's selection-style question tool instead of naming a
single API in the skill contract.

| Platform | Preferred tool |
|---|---|
| Codex | `request_user_input` (requires Plan mode) |
| Claude Code | `AskUserQuestionTool` |
| Other runtimes | Closest equivalent selection-style question UI |

If no selection UI exists in the current runtime, ask in plain text as
the exception path. Keep the same one-question-at-a-time discipline.
For Codex, use `request_user_input` for each high-impact question that
can be expressed honestly as 2-3 options. Do not switch back to plain
chat after the first answer while selection UI remains available.

## Flow

### Phase 1: Theme + search

1. Understand the user's concern or idea from their message.
2. Run `gwt-search` with 2-3 semantic queries (Japanese + English).
3. Check open SPEC Issues: `gwt issue spec list`.
4. If a clear owner exists, present it before going further.

### Phase 2: Investigation

**Do not ask the user anything until you have investigated.**

#### 2a. Read relevant code

- Grep for related functions, types, constants.
- Read the files that would be affected.
- If possible, run the code to observe actual behavior.

#### 2b. Dependency analysis

For every potential change surface, map:

| Column | Question |
|---|---|
| **Downstream** | What breaks if we change this? |
| **Upstream** | What must exist before this change is possible? |
| **Atomicity** | What must change in the same commit to avoid an intermediate broken state? |

Present as a table:

```markdown
| Change | Downstream impact | Upstream prerequisite | Must change together |
|---|---|---|---|
| ... | ... | ... | ... |
```

#### 2c. SPEC consistency check

- Compare implementation behavior against SPEC acceptance scenarios.
- Check data-model.md for undocumented entities or tables.
- Identify any acceptance scenario that doesn't match the code.

### Phase 3: Present findings

Show the user what you found:

- Tables comparing expected vs actual
- Dependency chains
- Code snippets with line references
- Gaps between SPEC and implementation

Do not propose a solution yet. Let the user absorb the findings first.

### Phase 4: Discuss

Ask the user one question at a time about the findings.
Wait for their answer before asking the next question.

Use the platform question tool first. Each question should:

- Use a selection UI when available
- Offer 2-3 mutually exclusive options
- Put the recommended option first
- Avoid freeform text unless options would distort the decision

After each answer:

- Update the Intake Memo
- Remove or refine the resolved unknown
- Re-rank the remaining unresolved questions
- Ask the next highest-impact question if any remain
- If a high-impact unknown remains and the platform question tool is
  still available, ask that next question via the same selection UI
  path before summarizing, handing off, or exiting

Prioritize questions by impact:

1. Questions that change the scope (in/out of scope)
2. Questions that change the approach (which solution path)
3. Questions that change sequencing (what order to do things)
4. Questions about naming or formatting (lowest priority)

### Phase 5: Repeat

Return to Phase 2-4 as many times as needed. There is no upper limit
on cycles. The discussion continues until:

- The user decides on an action (Phase 6)
- The user says "enough" or moves to another topic
- All open questions are resolved

Do not end the brainstorm after a single answer when unresolved
high-impact items still exist.
Do not emit the decision summary or hand off to another skill until all
high-impact unknowns are resolved or intentionally deferred in the
Intake Memo.

### Phase 6: Exit

Present the decision summary with your recommendation:

```text
## Brainstorm Decision

Path: NEW-SPEC | UPDATE-SPEC | ISSUE | CODE-FIX | LESSON | NO-ACTION
Owner: #<number> or "new"
Reason: <one sentence>

### Intake Memo
- <final bullets>

### Dependency Chain
- <key dependencies identified>

### SPEC Delta
- Section: <spec|plan|tasks>
  Change: <what will be added or changed>

### Proposed Action
- <specific next step>
```

**Exit paths:**

| Path | Handoff |
|---|---|
| NEW-SPEC | → `gwt-spec-design` (pass Intake Memo; design skips Phase 1) |
| UPDATE-SPEC | Update the owner SPEC artifact set in one batch (`spec` required, `tasks` / `plan` when the discussion changed them), then use `gwt-spec-design --deepen` only if more design work remains |
| ISSUE | → `gwt-issue` |
| CODE-FIX | → `gwt-spec-build` standalone (pass dependency chain as task context) |
| LESSON | → Write directly to `tasks/lessons.md` |
| NO-ACTION | → End with documented rationale |

**Wait for user approval before handing off.** Do not auto-proceed.

## Anti-patterns

- **Asking without investigating.** If you can find the answer by
  reading code, do that instead of asking the user.
- **Using plain text when selection UI exists.** If the runtime offers
  a question UI, use it instead of dumping text questions into the chat.
- **Ignoring dependencies.** Every change has downstream effects.
  Present the dependency chain before discussing the change itself.
- **Stopping after the first answer.** A brainstorm turn is not done
  until high-impact unknowns are resolved or intentionally deferred.
- **Updating SPECs incrementally mid-discussion.** Collect candidate
  deltas while discussing; apply them only once the direction is stable.
- **Writing code during brainstorm.** Brainstorm may update an owner
  SPEC at the end, but it should not drift into implementation. If you
  catch yourself writing code, stop and present the finding instead.
- **Defaulting to "let's make a SPEC."** Many brainstorms end with
  a code fix, a lessons.md entry, or "no action needed." That is
  fine. The goal is the right decision, not the most formal one.
- **Dumping all findings at once.** Present one finding, discuss it,
  then move to the next. The user needs time to think.
- **Committing in the middle of a dependency chain.** If the
  dependency analysis shows A+B+C must change together, do not
  commit A alone and leave B+C for "later."

## Relationship with other skills

```
gwt-spec-brainstorm (this skill)
  ├─ NEW-SPEC → gwt-spec-design (Phase 2+, Intake Memo handed off)
  ├─ UPDATE-SPEC → owner SPEC updated in one batch, then optionally gwt-spec-design --deepen
  ├─ ISSUE → gwt-issue
  ├─ CODE-FIX → gwt-spec-build (standalone)
  ├─ LESSON → tasks/lessons.md
  └─ NO-ACTION → end

gwt-spec-build can invoke this skill when:
  - Implementation reveals a spec-implementation mismatch
  - A dependency chain emerges that wasn't in tasks.md
  - The user questions an assumption during implementation
```
