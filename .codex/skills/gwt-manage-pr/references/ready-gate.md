## Ready PR Gate and Pre-PR Verification (mandatory)

Use the Ready PR Gate before any push, PR create, PR update, or state
change that would present the branch as Ready for review. A PR may be
Ready for review only when all of the following are true:

- the PR scope is a releaseable slice: it can be merged and shipped by
  itself without exposing incomplete behavior or breaking existing users
- all acceptance criteria for the PR scope are satisfied
- no known blockers remain for the PR scope
- rollback / follow-up boundaries are explained in the PR body when relevant
- `gwt-verify --mode pre-pr` returns `Overall: PASS`
- `User Verification Result` is `confirmed`, `n/a`, `n/a (autonomous)`,
  `deferred (autonomous execution)`, or `skipped(<reason>)`. For autonomous
  launches read `execution.status` (`launch_route`) and record
  `n/a (autonomous)`; UI work additionally needs `Agent Visual Check: pass`
  and actual passing headed Chromium dark/light evidence in the same fresh
  `verify.run` record (selected with `params.headed_e2e_commands`). Never
  rewrite the agent's own check as human `confirmed`.
- The legacy `deferred (autonomous execution)` value on existing autonomous
  PRs permits Ready with fresh passing evidence, without rewriting the body or
  asking for human confirmation. `pr.list` still exposes
  `deferred_user_verification` when `include: ["body"]` is requested: autonomous
  values, including legacy deferred, yield `false`; manual or generic deferred
  yields `true`. Without body hydration the field is absent. Manual verification
  is unchanged.
- every PR body checklist item is checked or explicitly marked N/A with
  a reason

Before Ready PR creation or enabling auto-merge, compare the owner Issue's
acceptance criteria with the fresh `verify.run` evidence. Update only proven
AC checkboxes through `issue.edit` (`issue.spec.edit` for a `gwt-spec` owner),
then read the owner back to confirm the saved state. Leave unproven criteria
unchecked, including delivery criteria until `origin/develop` contains the
source. Keep the same launch active through merge and post-merge settlement;
declare delivery complete only after confirming `origin/develop` contains the
source.

In gwt-managed execution worktrees the canonical operations enforce part of
this mechanically (SPEC-3248 P8b): non-draft `pr.create` and `pr.ready`
refuse when the session's Execution Control Record is terminally blocked, or
active without a fresh all-passing Verification Run Record (written by JSON
operation `verify.run`). Run the pre-PR matrix through `verify.run` so the
evidence exists before the Ready handoff; Draft PR creation stays available
mid-work.

Issue #4214 provides a separate Ready-creation exit when a terminal producer
cannot acquire local verification admission: `pr.create_from_ci` requires
explicit `base`, `head`, `head_sha`, `title`, `body` and optional `labels`.
The canonical operation fetches the repository's mandatory GitHub checks,
requires exact-head all-pass evidence, rechecks refs/policy before dispatch,
and audits its use without changing the Blocked execution or local test
results. Only the exact owning durable Session or the registered repository
PM may call it. A PM nominates the producer's head; the PM worktree's local
matrix cannot stand in for that head. Permission, measured UI evidence and
human/autonomous verification distinctions remain mandatory. Missing,
failed, stale or unreadable CI refuses creation. Never use a Draft PR or a
blanket Board waiver as this exit. Continue the existing auto-merge workflow
for the resulting Ready PR.

`gwt-verify` is a project-agnostic Generic Verification Contract: it
autodetects the host project's test runners (Cargo.toml / package.json /
pyproject.toml / go.mod / ProjectSettings / *.sln / etc.), runs the
appropriate unit / integration / E2E / visual tests for the changed
surfaces, emits a Test Inventory naming exactly which tests were
executed, and hands off to the user with a 4-step 導線 + Check Items
before finalizing the report. Project-local AGENTS.md / README testing
instructions always take precedence over the generic matrix.

Draft PR is the only allowed PR state for unfinished, partially
verified, acceptance-incomplete, or blocker-bearing work. Draft PR is
allowed for CI, shared visibility, and early review, but the PR body
must set `State: Draft`, list known blockers, list Remaining acceptance,
and avoid any claim that the change is complete or a releaseable slice.
Full `gwt-verify --mode pre-pr` is not required solely to create or
update a Draft PR, but the agent must include the exact verification
that was run and the reason full Ready verification is not yet claimed.

The Ready PR Gate **fails** when any of:

- `gwt-verify --mode pre-pr` returns `Overall: FAIL`
- `failed: tooling-missing` is recorded
- `User Verification Result` is `pending` (handoff never completed) or
  `rejected(<reason>)` (user declined). The user's reason is preserved
  in the evidence bundle.
- the PR body lists known blockers or Remaining acceptance for the PR
  scope
- the PR is not a releaseable slice

On Ready PR Gate failure, do not push a Ready/non-draft update, do not
create a Ready PR, and do not mark an existing Draft PR as Ready for
review. Either keep/create the PR as Draft PR with explicit blockers, or
return **NO ACTION** and route the failure for repair (back to the TDD
loop or `gwt-discussion`) before retrying.

This applies equally to Create and Fix modes. Every code-changing
re-push to a Ready/non-draft PR must be preceded by a fresh
`gwt-verify --mode pre-pr` PASS with a satisfied
`User Verification Result`.
