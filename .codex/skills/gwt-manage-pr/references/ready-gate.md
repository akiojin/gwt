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
- User Verification Result is optional. Require a human handoff only when the
  owner explicitly requests it; a missing, pending, or legacy deferred field
  alone does not create that request. Preserve an actual rejection and repair
  the reported defect. In autonomous launches, read `execution.status`
  (`launch_route`) and record `User Verification Result: n/a (autonomous)`.
- UI work needs `Agent Visual Check: pass` and actual passing headed Chromium
  dark/light evidence in the same fresh `verify.run` record (selected with
  `params.headed_e2e_commands`). Never rewrite the agent's own check as human
  `confirmed`.
- The legacy `deferred (autonomous execution)` value on existing autonomous
  PRs permits Ready with fresh passing evidence, without rewriting the body.
  `pr.list` exposes `deferred_user_verification` when `include: ["body"]` is
  requested; without body hydration the field is absent. This diagnostic
  does not itself establish an owner request for human verification.
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
evidence exists before the Ready handoff. Draft PRs are prohibited; repair
failed verification before creating a PR. For an inherited Draft, the
registered PM can call `pr.ready` against its exact verified head without
being refused solely because another session owns the verification record.
The target head, required CI, reviews, and measured UI evidence still apply.

Issue #4214 provides a separate Ready-creation exit when a terminal producer
cannot acquire local verification admission: `pr.create_from_ci` requires
explicit `base`, `head`, `head_sha`, `title`, `body` and optional `labels`.
The canonical operation fetches the repository's mandatory GitHub checks,
requires exact-head all-pass evidence, rechecks refs/policy before dispatch,
and audits its use without changing the Blocked execution or local test
results. Only the exact owning durable Session or the registered repository
PM may call it. A PM nominates the producer's head; the PM worktree's local
matrix cannot stand in for that head. Permission, measured UI evidence and
the owner's explicit verification requests remain mandatory. Missing,
failed, stale or unreadable CI refuses creation. Never use a Draft PR or a
blanket Board waiver as this exit. Continue the existing auto-merge workflow
for the resulting Ready PR.

`gwt-verify` is a project-agnostic Generic Verification Contract: it
autodetects the host project's test runners (Cargo.toml / package.json /
pyproject.toml / go.mod / ProjectSettings / *.sln / etc.), runs the
appropriate unit / integration / E2E / visual tests for the changed
surfaces, emits a Test Inventory naming exactly which tests were
executed, and provides a 4-step 導線 + Check Items when the owner explicitly
requests a human handoff. Project-local AGENTS.md / README testing
instructions always take precedence over the generic matrix.

Draft PRs are prohibited. Do not create or demote a PR to hold unfinished,
partially verified, acceptance-incomplete, or blocker-bearing work. Repair the
failure before creating a PR, or cut a smaller releaseable slice with a clear
follow-up boundary. An inherited Draft is recovered through `pr.ready`.

The Ready PR Gate **fails** when any of:

- `gwt-verify --mode pre-pr` returns `Overall: FAIL`
- `failed: tooling-missing` is recorded
- the owner explicitly requests human verification and that handoff is still
  `pending`
- `User Verification Result` records an actual `rejected(<reason>)`; preserve
  the reason and repair the known blocker
- the PR body lists known blockers or Remaining acceptance for the PR
  scope
- the PR is not a releaseable slice

On Ready PR Gate failure, do not push a Ready/non-draft update, do not
create a Ready PR, and do not mark an existing Draft PR as Ready for
review. Return **NO ACTION** with the failing gate item and route the failure
for repair (back to the TDD loop or `gwt-discussion`) before retrying.

This applies equally to Create and Fix modes. Every code-changing
re-push to a Ready/non-draft PR must be preceded by a fresh
`gwt-verify --mode pre-pr` PASS and any explicitly requested human check.
