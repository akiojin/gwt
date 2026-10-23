## Working artifacts

Keep these artifacts throughout the discussion:

- `Intake Memo` in conversation context for background, scope, and constraints
- `Discussion TODO` in conversation context and mirrored to the
  machine-local work-notes discussion log
  (`~/.gwt/projects/<repo-hash>/work-notes/discussions.md`, SPEC-3214);
  the repo-local `.gwt/work/discussions.md` and legacy `.gwt/discussion.md`
  are read fallbacks only
- `Action Delta` for changes that are ready to land in `spec`, `plan`, `issue`,
  or build context
- `Action Bundle` for the concrete follow-up actions that should happen next

When the discussed work touches multiple Issues/SPECs, the artifact update is
not complete until exactly one Primary Execution Owner is selected and every
related owner is classified (SPEC-3248 Multi-Issue Primary Owner Bundle):

- `bundled-required` — must ship in the Primary owner's PR; copy or summarize
  its required tasks and acceptance into the Primary so a single-Issue
  Execution launch can implement the full bundled scope without opening every
  related Issue
- `dependent-follow-up` — separately deliverable with explicit ordering and
  blocker notes; never a hidden Primary completion requirement
- `reference-only` — background reading, not part of the PR's completion

Read `Issue relationship markers` in the generated `gwt-coordination` skill's
`references/work.md` and persist that classification using its shared contract
before declaring the artifact update complete. Read back every changed
owner body or SPEC section; prose in the discussion or Primary alone does not
establish a dependent's `Blocked-by:` relationship. Existing mutation authority
and discussion approval boundaries still apply.

`Discussion TODO` is not an implementation task list. It tracks unresolved
questions, dependency checks, deferred decisions, coverage gaps, exit blockers,
the next question to ask, and the depth state for the active proposal.

Evidence is part of the discussion state, not a later implementation detail.
Do not mark a proposal `[chosen]` until its evidence fields prove the outcome
without relying on speculation, guesses, or vibes.

Mirror structure inside an active entry in the work-notes discussion log:

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
- Depth Mode:
- Question Ledger:
- Depth Gate:
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
which inspects the work-notes discussion log and falls back to the
repo-local `.gwt/work/discussions.md`, then legacy `.gwt/discussion.md`,
then blocks Stop (with
`{"decision":"block","reason":"..."}`) while any proposal is still
`[active]` with a non-empty `Next Question:`, unresolved `Exit Blockers:`,
incomplete proof fields, incomplete depth fields, or an `Evidence Gate:` that
is not `complete`. Depth fields are incomplete when `Question Ledger:` is
empty, `Depth Gate:` is missing/open, or `Depth Gate: deferred(<reason>)` has
no reason. To let Stop succeed, mark each proposal explicitly using the exit
CLI:

- `discuss.resolve` JSON operation — active → chosen only after
  `Evidence Gate: complete`
- `discuss.park` JSON operation — active → parked (resume later)
- `discuss.reject` JSON operation — active → rejected
- `discuss.clear_next_question` JSON operation — clear only the
  question line. It does not bypass incomplete evidence.

When the `Action Bundle` is produced, call the matching exit command for each
proposal before stopping. The runtime's `stop_hook_active` flag (built into
Claude Code; shared with Codex via `codex_hooks`) keeps the handler fail-safe:
at most one forced continuation per Stop cycle.

The Stop gate is session-scoped (Issue #3465). The work-notes discussion log is
machine-local but project-scoped, so every session of the project reads the
same entries. `discussion.update` stamps the opening session into the entry
metadata as `Origin Session: <GWT_SESSION_ID>`, and `skill-discussion-stop-check`
only blocks on entries whose `Origin Session` matches the current session. An
entry that records no `Origin Session`, or a session launched without
`GWT_SESSION_ID`, keeps the previous fail-closed behaviour. Never run
`discuss.park` / `resolve` / `reject` on another session's proposal just to free
your own Stop — it rewrites the owner's in-progress state.

Open a discussion entry with `discussion.update` before writing the
`Discussion TODO` proposals, so the entry carries its `Origin Session` stamp.

## Resume hooks

Managed hook settings in `.claude/settings.local.json` and `.codex/hooks.json`
may surface unfinished discussion candidates from the work-notes discussion
log with repo-local `.gwt/work/discussions.md` and legacy `.gwt/discussion.md`
fallbacks.

Use this contract:

- Treat a proposal marked `[active]` with a non-empty `Next Question` as
  resumable.
- Prefer `SessionStart` to surface the resume prompt.
- If the runtime does not emit `SessionStart` before the first turn, arm a
  fallback from `UserPromptSubmit` and surface it after the next `Stop`.
- Offer exactly `Resume discussion`, `Park proposal`, and `Dismiss for now`.
- `Resume discussion` continues `gwt-discussion` before other work proceeds.
- `Park proposal` changes the matching proposal in the work-notes discussion log
  from `[active]` to `[parked]`.
- `Dismiss for now` suppresses the prompt only for the current agent session. A
  later session may surface it again.
- `PreToolUse` may also dispatch `workflow-policy` before mutating tool calls.
- `workflow-policy` should allow read-only investigation, but block
  implementation edits until an owner Issue or SPEC is linked.
- If the owner is a `gwt-spec` Issue, `workflow-policy` should block
  implementation until the owner SPEC cache has non-empty `plan` and `tasks`
  sections; local files alone do not unblock it.

## Platform question tool

Use an available selection-style question tool when it helps resolve a necessary choice; plain conversation is also suitable. Respect launch restrictions on questions.

| Platform | Preferred tool |
|---|---|
| Codex | `request_user_input` |
| Claude Code | `AskUserQuestionTool` |
| Other runtimes | Closest equivalent selection-style question UI |

If no selection UI exists, ask in plain text when an answer is needed. Keep questions focused and proportional to the uncertainty.
