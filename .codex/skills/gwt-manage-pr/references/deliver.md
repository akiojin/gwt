## Mode: Deliver (drive-to-merge)

Detailed logic in [references/deliver-flow.md](deliver-flow.md).

Deliver mode drives a verified, Ready PR all the way to a merged state: hand
the PR to the repository's merge automation, then run the existing Fix loop
against every blocker (CI / reviews / threads / conflicts) and poll until
`merged_at` is set. Deliver **composes** Fix mode — it does not reimplement
blocker resolution.

gwtd has no merge operation. The merge itself is performed by the
repository's merge automation (GitHub auto-merge, or a workflow that merges
non-Draft PRs once checks pass) or by a human. JSON
operation `pr.ready` hands an inherited Draft PR to that automation.
Draft PRs are prohibited; creation and demotion are unavailable.

### Entry: autonomous execution or explicit manual request

For `launch_route: autonomous`, Deliver continues verified work through a
Ready PR and the existing CI auto-merge path until merged. Human visual
confirmation is not required, and a Draft PR is not the terminal outcome for
passing work. Manual Deliver is opt-in only and never auto-routed: the user must
explicitly ask to deliver / drive to merge / merge / land / ship the PR. If no
open PR exists, first use Create with the Ready PR Gate, then drive that PR.

### Hard PR Gate (mandatory before handing the PR to merge automation)

Do **not** hand the PR to merge automation until the Ready PR Gate passes:

- `gwt-verify --mode pre-pr` returns `Overall: PASS`.
- User Verification Result is optional unless the owner explicitly requests
  a human check. An absent, `pending`, or legacy deferred value alone does not
  establish such a request. An actual `rejected(<reason>)` records a known
  blocker that must be repaired. Keep autonomous `n/a (autonomous)` and legacy
  `deferred (autonomous execution)` compatibility without rewriting the body.
- UI work requires `Agent Visual Check: pass` plus the same fresh record's
  measured passing headed Chromium results for dark and light themes.
- The PR is a releaseable slice with no known blockers.

If automated verification fails or an explicitly requested check is pending,
repair the failure before handing new work over through `pr.ready`.
Never turn an agent check into human `confirmed` or invent a skip result.

### Preserve the repository's delivery policy

Keep auto-merge enabled across routine pushes and base synchronization. Run
fresh `gwt-verify --mode pre-pr` before every code-changing push. Use JSON
operation `pr.update_branch` for a BEHIND PR; when the PM owns synchronization,
hand it off on the Board instead of updating the branch concurrently.

Do not require an unavailable merge operation or a Draft/Ready cycle for each
push. Draft creation and demotion are unavailable; repair any failed gate
before the next code-changing push.

### Drive-to-merge loop

1. Preflight + Hard PR Gate (above). Resolve the PR with `pr.current` or the
   user-supplied number; read state with `pr.view` (`[MERGED]` bracket plus
   `mergeable:` / `merge_state:` / `ci:` / `review:` lines).
2. Inspect with Fix `--mode all` and resolve **every** BLOCKING item through
   the existing Fix Implementation / Comment Response flow. Re-run the Hard
   PR Gate before each code-changing push.
3. Confirm the PR has merge automation (GitHub auto-merge enabled on it, or a
   repository workflow that merges non-Draft PRs). Without it the merge needs
   a human: report the clear, gated PR and stop, because gwtd cannot merge it.
4. Hand over **only a verified snapshot**: if an inherited PR is Draft, use
   JSON operation `pr.ready` after the gate passes. Preserve enabled auto-merge.
5. Poll `pr.view` ~30s. Resolve any **new** blocker (BEHIND, new failing check,
   new thread/CHANGES_REQUESTED) through Fix while preserving auto-merge.
   Re-run only **infrastructure-transient** CI failures with
   JSON operation `actions.rerun` for the failed jobs (max 3); a test/build timeout or compile/
   test failure is code, not transient — fix it. The poll is bounded (~20 polls
   / ~10 min) then hands off via `board.post`.
6. Continue until `pr.view` shows `[MERGED]` (GitHub `merged_at` set), then
   report `Delivered`. "Handed to merge automation" is not "merged."

### Loop Safety Guard

CI pending/queued is a temporary wait; pending alone never counts toward the
3 consecutive failed drive iterations. Read `execution.status` / `launch_route`:
an autonomous / Issue Monitor launch must not ask a human to continue merely
because the same checks are still running.

At the bounded polling limit, post `board.post` with `params.kind:"status"`
or `params.kind:"handoff"`, the PR number, pending check names, and a resume
instruction. Declare `issue.monitor.wait` with the explicit owner Issue number
in `params.number`, a `reason`, and a `resume_condition` naming the pending CI.
This ends the polling batch, not the execution: keep it Active, preserve
auto-merge, and resume the bounded check path when CI progresses. Clear the
declaration with `issue.monitor.wait` and `params.clear:true` when resuming.
Without a Monitor owner, use the status/handoff without inventing an Issue.
Do not emit `params.kind:"blocked"` or NeedsHuman, call `execution.blocked`,
or claim Delivered for a pending-only wait. Ready PR, required CI, and
auto-merge gates remain unchanged; delivery still requires `merged_at`.

A real blocker (the same failed CI check, unresolved thread, or conflict)
surviving 3 consecutive repair iterations still stops the repair loop: report
what was tried and obtain the required **continue** / **abort** / **change
approach** decision. A human decision, unsafe operation, or actual tool refusal
still follows the existing blocked escalation contract in `gwt-coordination`.

### Optional: arm a completion goal (SPEC-3050)

Driving to merge can span CI runs longer than one turn. Optionally arm a "PR
merged" completion goal so monitoring survives turn boundaries, using the same
goal-start contract as `/release` step 5.4 (Codex `create_goal`; Claude Code
`pane.send` of `/goal <condition>`). Arming the goal is best effort and never
replaces the in-loop `pr.view` poll.

## Diagnosis Report Anti-Patterns

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 blocking items detected" |
| "This might be causing..." | "Root cause: `<error from log>`" |
| "Consider fixing..." | "Action: Fix `<what>` in `<where>`" |
| "Various CI checks are failing" | "2 CI checks failing: `build`, `lint`" |

## Comment Formatting Rules

- No escaped newline literals (`\n`) in final comment text.
- Use real line breaks. Verify before posting.
- If raw escape sequences needed for explanation, use fenced code blocks only.

## Issue Progress Comment Template

When work is tracked in Issues:

```markdown
Progress
- ...

Done
- ...

Next
- ...
```

## Bundled Scripts

PR lifecycle work is driven through canonical gwtd JSON operations `pr.*` and
`actions.*`. Do not depend on retired repo-local helper script
paths when running this workflow.

## References

- [references/create-flow.md](create-flow.md) -- PR creation workflow details
- [references/check-flow.md](check-flow.md) -- PR status checking details
- [references/fix-flow.md](fix-flow.md) -- CI/conflict/review fix details
- [references/deliver-flow.md](deliver-flow.md) -- drive-to-merge (auto-merge + Fix loop + merged watch) details
- [references/pr-body-template.md](pr-body-template.md) -- PR body template
