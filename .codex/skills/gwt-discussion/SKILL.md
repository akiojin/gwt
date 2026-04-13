---
name: gwt-discussion
description: "Use when an idea, spec question, or implementation gap needs investigation and discussion before deciding how work should proceed."
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: "[idea | concern | implementation gap | --deepen SPEC-N]"
---

# gwt-discussion

Unified discussion entrypoint for exploration, design clarification, and
mid-implementation investigation. Use it before a SPEC exists, while a SPEC or
plan is being refined, or when implementation reveals a gap that needs user
discussion before work can continue.

Use the current user's language for decision summaries, `Discussion TODO`,
`Action Delta`, `Action Bundle`, and any user-facing artifact text generated
during this workflow, unless an existing artifact must keep its established
language.

## When to use

- A rough idea may or may not become a SPEC
- An existing SPEC or plan feels incomplete or wrong
- Mid-implementation work uncovered a gap, inconsistency, or dependency chain
- The next step is unclear and needs investigation before `gwt-plan-spec` or
  `gwt-build-spec`

Do not use this when the work is already decision-complete and ready for
planning or implementation. Use `gwt-plan-spec` or `gwt-build-spec` directly in
that case.

## Working artifacts

Keep these artifacts throughout the discussion:

- `Intake Memo` in conversation context for background, scope, and constraints
- `Discussion TODO` in conversation context and mirrored to
  `.gwt/discussion.md`
- `Action Delta` for changes that are ready to land in `spec`, `plan`, `issue`,
  or build context
- `Action Bundle` for the concrete follow-up actions that should happen next

`Discussion TODO` is not an implementation task list. It tracks unresolved
questions, dependency checks, deferred decisions, and the next question to ask.

Mirror structure for `.gwt/discussion.md`:

```markdown
## Discussion TODO

### Proposal A - <title> [active|parked|rejected|chosen]
- Summary:
- Open Questions:
- Dependency Checks:
- Deferred Decisions:
- Next Question:
- Promotable Changes:
```

Promote an item from `Discussion TODO` into `Action Delta` only after the
high-impact unknown behind it has been resolved.

## Resume hooks

Managed hook settings in `.claude/settings.local.json` and `.codex/hooks.json`
may surface unfinished discussion candidates from `.gwt/discussion.md`.

Use this contract:

- Treat a proposal marked `[active]` with a non-empty `Next Question` as
  resumable.
- Prefer `SessionStart` to surface the resume prompt.
- If the runtime does not emit `SessionStart` before the first turn, arm a
  fallback from `UserPromptSubmit` and surface it after the next `Stop`.
- Offer exactly `Resume discussion`, `Park proposal`, and `Dismiss for now`.
- `Resume discussion` continues `gwt-discussion` before other work proceeds.
- `Park proposal` changes the matching proposal in `.gwt/discussion.md` from
  `[active]` to `[parked]`.
- `Dismiss for now` suppresses the prompt only for the current agent session. A
  later session may surface it again.
- `PreToolUse` may also dispatch `workflow-policy` before mutating tool calls.
- `workflow-policy` should allow read-only investigation, but block
  implementation edits until an owner Issue or SPEC is linked.
- If the owner is a `gwt-spec` Issue, `workflow-policy` should block
  implementation until the owner SPEC cache has non-empty `plan` and `tasks`
  sections; local files alone do not unblock it.

## Platform question tool

Use the platform's selection-style question tool instead of naming a single API
in the contract.

| Platform | Preferred tool |
|---|---|
| Codex | `request_user_input` |
| Claude Code | `AskUserQuestionTool` |
| Other runtimes | Closest equivalent selection-style question UI |

If no selection UI exists in the current runtime, ask in plain text as the
exception path. Keep the same one-question-at-a-time discipline.

## Flow

### Phase 1: Theme + search

1. Understand the user's concern, idea, or implementation gap.
2. Run `gwt-search` with 2-3 semantic queries in Japanese and English.
3. Check open SPEC Issues: `gwt issue spec list`.
4. If an existing Issue number or URL is already the primary owner, capture it
   in the `Intake Memo`.
5. If a clear owner exists, present it before going further.

### Phase 2: Investigation

Do not ask the user anything until you have investigated.

- Grep for related functions, types, constants, and commands
- Read the files or artifacts that would change
- If possible, run the code or command to observe actual behavior
- Compare code, SPEC, plan, and issue state when they disagree

For every potential change surface, map:

```markdown
| Change | Downstream impact | Upstream prerequisite | Must change together |
|---|---|---|---|
| ... | ... | ... | ... |
```

### Phase 3: Discuss

Present findings first:

- expected vs actual behavior
- dependency chains
- gaps between SPEC, plan, issue, and code
- which proposal is currently `active`, `parked`, `rejected`, or `chosen`

Ask the user one question at a time about the findings.
Wait for their answer before asking the next question.

Use the platform question tool first. Each question should:

- use a selection UI when available
- offer 2-3 mutually exclusive options
- put the recommended option first
- avoid freeform text unless options would distort the decision

After each answer:

- update the `Intake Memo`
- update the `Discussion TODO`
- remove or refine the resolved unknown
- re-rank the remaining unresolved questions
- Ask the next highest-impact question if any remain

When the runtime and user allow delegation, a bounded subagent may be used for
objective review of competing proposals. Keep that subagent scoped to
independent comparison work and do not let it replace the main discussion flow.

Do not end the discussion after a single answer when unresolved high-impact
unknowns still exist.

### Phase 4: Design and artifact updates

When the discussion stabilizes, update the right artifacts in one batch.

#### If new or updated behavior needs a SPEC

- Use `references/intake.md` for search and routing discipline
- Use `references/ddd-modeling.md` for Bounded Context and domain modeling
- Use `references/registration.md` to create or seed `spec.md`
- Use `references/clarification.md` to remove high-impact
  `[NEEDS CLARIFICATION]` markers
- Use `references/deepening.md` when the user asks for deeper analysis on an
  existing SPEC

#### If the plan is stale but the SPEC is already settled

- Update the owner `plan` and related planning artifacts in-place
- Capture the confirmed deltas in `Action Delta`
- Return `gwt-plan-spec` in the `Action Bundle` when further planning work is
  still needed

#### If implementation can resume

- Update the owner `spec` / `plan` / `issue` context as needed first
- Return `Resume Build` in the `Action Bundle`
- Let `gwt-build-spec` continue with the updated context

#### If the result is narrower than a SPEC

- Return `Update Issue` or `Write Lesson` in the `Action Bundle`
- Use `gwt-register-issue` or `gwt-fix-issue` when the GitHub Issue flow should
  own the next step

### Phase 5: Exit

Present the result in this format:

```text
## <Discussion Decision in the current user's language>

Owner: #<number> | SPEC-<id> | "new" | "none"
Reason: <one sentence>

### Intake Memo
- <final bullets>

### Discussion TODO
- Proposal A [active|parked|rejected|chosen]: <state + remaining concern>

### Action Delta
- Update Spec: <what changes>
- Update Plan: <what changes>
- Update Issue: <what changes>
- Resume Build Context: <what the implementer must use>

### Action Bundle
- Update Spec
- Update Plan
- Resume Build
- Update Issue
- Write Lesson
- No Action
```

`Action Bundle` may contain multiple actions. Examples:

- `Update Spec` + `Update Plan` + `Resume Build`
- `Update Issue` + `No Action`
- `Write Lesson` only

## Routing notes

- Return `gwt-plan-spec` when design is stable but planning work remains
- Return `gwt-build-spec` when implementation should continue
- Return `gwt-register-issue` when new work should become an Issue instead of a
  SPEC
- Return `gwt-fix-issue` when an existing Issue owns the next step

## Anti-patterns

- Asking before investigating
- Updating artifacts incrementally in the middle of unresolved discussion
- Turning `Discussion TODO` into an implementation task list
- Ending after the first answer when high-impact unknowns remain
- Writing code during discussion instead of returning an `Action Bundle`
