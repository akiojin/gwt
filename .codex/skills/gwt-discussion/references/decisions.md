### Phase 4: Design and artifact updates

When the discussion stabilizes, update the right artifacts in one batch.

#### If new or updated behavior needs a SPEC

- Use [references/intake.md](intake.md) for search and routing discipline
- Use [references/ddd-modeling.md](ddd-modeling.md) for Bounded Context and domain modeling
- **Register the SPEC via `gwt-register-spec`** (sub-skill, SPEC-2784). The
  caller prepares title + body file from this discussion's outcome; the
  sub-skill validates, executes the canonical `issue.spec.create` →
  `issue.spec.edit` → `issue.spec.section` roundtrip safely, and returns the new Issue number. Add
  `Register Spec` to the Action Bundle. Use [references/registration.md](registration.md)
  only when the sub-skill is unavailable; in that case follow its 2-step
  flow manually and verify `issue.spec.section` returns non-empty content
  before handoff.
- Use [references/clarification.md](clarification.md) to remove high-impact
  `[NEEDS CLARIFICATION]` markers
- Use [references/deepening.md](deepening.md) when the user asks for deeper analysis on an
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

A useful result format is:

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
- Register Spec
- Update Spec
- Update Plan
- Resume Build
- Update Issue
- Write Memory
- No Action
```

`Register Spec` runs the `gwt-register-spec` sub-skill (SPEC-2784) to
materialize a new SPEC Issue safely. Use it instead of manual
`issue.spec.create` whenever a fresh SPEC owner is needed.

This final result is the handoff point where the workflow may leave Plan Mode.

`Action Bundle` may contain multiple actions. Examples:

- `Update Spec` + `Update Plan` + `Resume Build`
- `Update Issue` + `No Action`
- `Write Memory` only

### Goal Start (after Action Bundle approval)

When the Action Bundle is ready and follow-up work can continue
autonomously after user approval, arm a runtime goal from the bundle
(SPEC-3050):

1. Build a goal condition from the Action Bundle: one verifiable end state
   (tests green, lint clean, evidence bundle emitted), the check that proves
   it, and a turn or time cap (for example "or stop after 20 turns"). The
   condition must respect gwt's PR gate: stop at "verification handoff ready
   with a recorded User Verification Result" — never "PR created" or "PR
   merged".
2. Before the final discussion response stops, write the condition to a temp
   file and run JSON operation `discuss.goal_pending`
   after resolving the chosen proposal. This records `Goal State: pending`
   in the work-notes discussion log so the next `UserPromptSubmit` hook can remind
   the agent even though the approval arrives in a later turn.
3. In the next turn, if the user approves the Action Bundle and asks the
   follow-up work to continue autonomously, start the goal per runtime:
   - **Codex** (goals enabled; gwt launches Codex with `--enable goals`):
     call `create_goal` with the condition as the objective — the goals tool
     contract allows the model to start a Goal itself.
   - **Claude Code** (v2.1.139 or later): the built-in `/goal` command cannot
     be self-invoked by the agent. Queue it into your own pane instead:
     JSON operation `pane.send` with `params.text:"/goal <condition>"` (resolve `GWT_BIN` first per
     the gwtd resolution section). The injected line is submitted
     automatically when the current turn ends. `pane send` is self-only: it
     targets the pane bound to `GWT_SESSION_ID` and rejects other panes.
4. After a successful goal start, run
   JSON operation `discuss.goal_started`. If the user rejects or
   revises the Action Bundle instead, run JSON operation
   `discuss.goal_skipped`
   before continuing the discussion. If the goal cannot be started (older
   Claude Code, trust dialog not accepted, goals feature disabled,
   `pane.send` failure), run JSON operation `discuss.goal_failed`,
   report the failure reason explicitly, print the assembled `/goal
   <condition>` line so the user can run it manually, and continue the exit.
   Goal start failures never block the discussion exit.

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
