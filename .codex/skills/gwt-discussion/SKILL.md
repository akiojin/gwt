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

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

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
questions, dependency checks, deferred decisions, coverage gaps, exit blockers,
and the next question to ask.

Evidence is part of the discussion state, not a later implementation detail.
Do not mark a proposal `[chosen]` until its evidence fields prove the outcome
without relying on speculation, guesses, or vibes.

Mirror structure for `.gwt/discussion.md`:

```markdown
## Discussion TODO

### Proposal A - <title> [active|parked|rejected|chosen]
- Summary:
- Open Questions:
- Dependency Checks:
- Deferred Decisions:
- Coverage Checks:
- Exit Blockers:
- Next Question:
- Implementation Proof:
- SPEC/Issue Proof:
- Gap Check Proof:
- Official Docs Proof:
- External Research Proof:
- Evidence Gate:
- Promotable Changes:
```

Promote an item from `Discussion TODO` into `Action Delta` only after the
high-impact unknown behind it has been resolved.

## Mode contract

- Start the discussion in Plan Mode. If the caller is not already in Plan
  Mode, enter it before Phase 1 begins.
- Resume paths should also re-enter Plan Mode before continuing the
  discussion.
- Stay in Plan Mode while investigation, questioning, and artifact shaping are
  in progress.
- The final discussion result should assume the workflow is about to leave
  Plan Mode.
- Do not leave Plan Mode before the official discussion result (`Action Delta`
  and `Action Bundle`, or a decision-complete `<proposed_plan>` when the
  discussion ends in a plan handoff) is ready.

## Exit CLI (Stop-block contract)

SPEC-1935 FR-014p routes Stop events through `skill-discussion-stop-check`,
which inspects `.gwt/discussion.md` and blocks Stop (with
`{"decision":"block","reason":"..."}`) while any proposal is still `[active]`
with a non-empty `Next Question:`, unresolved `Exit Blockers:`, incomplete
proof fields, or an `Evidence Gate:` that is not `complete`. To let Stop
succeed, mark each proposal explicitly using the exit CLI:

- `gwtd discuss resolve --proposal "Proposal A"` — active → chosen only after
  `Evidence Gate: complete`
- `gwtd discuss park --proposal "Proposal A"` — active → parked (resume later)
- `gwtd discuss reject --proposal "Proposal A"` — active → rejected
- `gwtd discuss clear-next-question --proposal "Proposal A"` — clear only the
  question line. It does not bypass incomplete evidence.

When the `Action Bundle` is produced, call the matching exit command for each
proposal before stopping. Codex's `stop_hook_active` flag (shared with Claude
Code via `codex_hooks`) keeps the handler fail-safe: at most one forced
continuation per Stop cycle.

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
3. Check open SPEC Issues: `gwtd issue spec list`.
4. If an existing Issue number or URL is already the primary owner, capture it
   in the `Intake Memo`.
5. If a clear owner exists, present it before going further.
6. If the clear owner is an existing SPEC, run the Board active-claim preflight
   before changing `.gwt/discussion.md` or any SPEC/plan artifacts:
   - Read the current Board with `gwtd board show --json` when available, or
     `gwtd board show` as the fallback.
   - Look for active `claim` entries from another session that mention the same
     owner (`#<N>` or `SPEC-<N>`) or the same phase/topic under discussion.
   - If a matching claim exists, pause the discussion flow and present the
     conflict: join that session with a Board handoff request, redesign a
     disjoint work split, or continue only after the user explicitly accepts
     duplicate risk.
   - Intentional parallel discussion/planning is allowed only when ownership is
     disjoint. Post a fresh Board `claim` with a `Boundary:` line naming the
     topic, artifacts, or files owned by this session before continuing.
   - Acceptance scenario: Given another session has an active Board claim for
     `SPEC-2008 Phase 24`, when `gwt-discussion --deepen SPEC-2008` starts,
     then the preflight reports the claim and requires user confirmation before
     producing an Action Delta or changing SPEC artifacts.

### Phase 2: Investigation

Do not ask the user anything until you have investigated.

- Grep for related functions, types, constants, and commands
- Read the files or artifacts that would change
- If possible, run the code or command to observe actual behavior
- Compare code, SPEC, plan, and issue state when they disagree
- Check official documentation for external APIs, runtimes, operating systems,
  CLIs, libraries, hooks, or platform behavior that cannot be proven locally
- Use web research when repo-local evidence and official documentation are not
  enough. For fast-moving operational facts, use X search when available, and
  record the source account, URL, timestamp, and whether it is first-party

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

## Discussion Depth Gate

Do not stop because one answer landed or because an arbitrary question count
was reached. Re-run this gate after each answer.

Coverage Checks:

- scope boundary
- ownership / integration
- failure / edge case
- migration / compatibility
- verification / success signal

Exit Blockers:

- a high-impact unknown still changes implementation, routing, or ownership
- an applicable coverage category above has not been addressed
- `Open Questions`, `Dependency Checks`, or `Deferred Decisions` still contain
  unresolved high-impact items
- the latest answer introduced a new high-impact unknown

Question depth ladder:

1. Direction / proposal choice
2. Boundary, exceptions, and failure handling
3. Acceptance, verification, and implementation-affecting detail

Keep asking one question at a time until the relevant coverage categories are
closed or explicitly deferred with rationale in `Discussion TODO`.

When the runtime and user allow delegation, a bounded subagent may be used for
objective review of competing proposals. Keep that subagent scoped to
independent comparison work and do not let it replace the main discussion flow.

Do not end the discussion after a single answer when unresolved high-impact
unknowns still exist.

## Evidence Gate

Before producing the final Action Delta / Action Bundle, complete the proof
fields for every `[active]` proposal that will become `[chosen]`.

- `Implementation Proof`: name the implementation files, logs, commands, or
  tests inspected or run. If no implementation exists, say what proves that.
- `SPEC/Issue Proof`: name the owner SPEC / plan / tasks / Issue sections read,
  including any missing or stale items found.
- `Gap Check Proof`: explicitly cover scope, ownership/integration,
  failure/edge cases, migration/compatibility, and verification/success signal.
- `Official Docs Proof`: cite official documentation checked for external API,
  runtime, OS, CLI, library, or hook behavior. Use `not-applicable: <reason>`
  only for purely local behavior.
- `External Research Proof`: cite non-official research, issue trackers, web
  search, or X search when needed. Use `not-applicable: <reason>` only when
  official docs and local evidence fully settle the question.
- `Evidence Gate`: set to `complete` only when the proposal has no unresolved
  `Exit Blockers:` and all proof fields above are non-empty or explicitly
  `not-applicable: <reason>`.

Official documentation is the preferred external evidence. X search and
X API Search Posts official docs are valid discovery paths for fast-moving
facts, but X posts alone are not sufficient for irreversible conclusions unless
they are first-party posts with URL and timestamp, or are corroborated by
official docs or another primary source.

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

- Return `Update Issue` or `Write Memory` in the `Action Bundle`
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
- Write Memory
- No Action
```

This final result is the handoff point where the workflow may leave Plan Mode.

`Action Bundle` may contain multiple actions. Examples:

- `Update Spec` + `Update Plan` + `Resume Build`
- `Update Issue` + `No Action`
- `Write Memory` only

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
