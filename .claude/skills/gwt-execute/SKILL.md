---
name: gwt-execute
description: "Use when execution should start from a GitHub Issue, gwt-spec Issue, or approved standalone task through one build/test/verify loop."
---

# gwt-execute

Unified Execute-lane entrypoint for implementation work. Treat every owner as a
Work Item: a GitHub Issue number plus labels and artifacts. `gwt-spec` is a
design-required tag, not a separate execution kind.

## Execution ownership

Once started, keep moving until one of these is true:

- the scoped tasks are complete
- a real product or scope decision blocks the next task
- a merge conflict or reviewer request cannot be resolved with high confidence
- required auth or tooling is unavailable
- a spec-implementation mismatch is discovered that changes the design surface
  (not just a typo fix) - use `gwt-discussion` to investigate and discuss

Use the current user's language for task summaries, completion reports, task
check updates, and any user-facing text generated while executing the workflow,
unless an existing artifact must keep its established language.

## gwtd resolution

Before executing any `gwtd ...` command from this skill, resolve `GWT_BIN`
first: executable `GWT_BIN_PATH`, then `command -v gwtd`, then
`$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the command
as `"$GWT_BIN" ...`; if none exists, stop with an actionable `gwtd not found`
error.

## Lifecycle

Use the existing build lifecycle JSON operations for Issue-backed work.
The operation names and state file remain compatibility surfaces. The `spec`
parameter carries the owner Issue number; it does not require a `gwt-spec`
label. Keep that same number throughout the lifecycle.

- `build.start` with `params.spec:<n>` for every Issue owner, including plain
  Issues in direct mode.
- `build.phase` with the same `params.spec:<n>` and
  `params.label:"red"|"green"|"refactor"|"verify"|"pr"` at each TDD milestone.
- `build.complete` with the same `params.spec:<n>` only after verification
  passed and the Ready PR Gate is satisfied for a releaseable slice.
- `build.abort` with the same `params.spec:<n>` and a concrete `params.reason`
  when implementation cannot proceed.

Without an owner Issue, do not call `build.*` or invent an Issue number.
Standalone work follows the approved task, TDD, and applicable verification
workflow below without this Issue-bound lifecycle. If the work acquires a
durable owner under the existing ownership rules, use that Issue number.

If an active build lifecycle exists, run `build.abort` with the same owner and a non-empty reason before `execution.blocked`.

Linked-owner Execution launches also carry an Execution Control Record
(SPEC-3248 P8a) written at launch, and Stop stays blocked until the record is
settled — even when `build.start` was never called (plain-Issue fixes
included):

- done and verified: JSON operation `execution.complete` (a successful
  `build.complete` settles the record too), or
- blocked by the environment or missing verification: JSON operation
  `execution.blocked` with a non-empty `params.reason` and optional
  `params.missing_verification`. Blocked is not done — report the blocker.

`execution.blocked` is a terminal outcome, not a pause. Never use it while
waiting for a temporary question, owner decision, or verification that can
still proceed in the current execution. Continue the discussion/workflow
instead.

If the same owning session later resolves a genuine terminal blocker, recover
without rewriting trusted state or relaunching: register the changed-surface
matrix through `verify.plan` with `params.derive:true`, run the entire
post-block matrix through `verify.run`, then call `execution.reopen` with a
non-empty `params.reason`. Reopen requires the exact immutable derived-plan
snapshot, fresh passing evidence that started after the block, the same
session/owner/worktree fingerprint, and valid integrity hashes. It appends a
recovery audit entry and returns the execution to Active; it does not claim
completion. Completed executions remain immutable.

When a worktree inherits a terminal Blocked record from a Session that is gone
— the startup reaper settles a defunct generation, and the relaunch lands in
the same worktree — the relaunched Session takes the record over with audited
`execution.adopt` first. Adoption transfers ownership without changing the
lifecycle, so the record stays Blocked and the same-session route above
(`verify.plan` `params.derive:true`, `verify.run`, `execution.reopen`) then
applies. Adoption is refused while the previous holder Session is still
running. Completed records are never adopted; another session must use a fresh
linked-owner launch.

Completion and Ready PR handoffs consume tool-generated verification
evidence (SPEC-3248 P8b): run the verification matrix through JSON operation
`verify.plan` first, then through `verify.run` with `params.commands:[...]` so
gwtd itself executes and records it. `execution.complete`, the execution
settlement inside `build.complete`,
non-draft `pr.create`, and `pr.ready` refuse when the latest record is
missing, failing, stale (worktree changed after the run), from another
session, or for another owner. Draft PRs stay available mid-work.

When resuming or recovering another session's execution (crash, closed
window), take over the record explicitly with JSON operation
`execution.adopt` and a non-empty `params.reason` — takeovers are audited as
an ownership transfer chain. Adopt requires a valid integrity record. An
integrity-failed record is repaired in place with JSON operation
`execution.repair`: it quarantines the corrupt record under a unique
`.corrupt-*` path with a trusted audit entry and atomically materializes a
fresh Active record, so the same execution lifetime can continue. Diagnose
first with `execution.status` — its `available_recoveries` names the exact
operation to run.

When a launch is refused with `... refuses while a Prepared successor or
takeover targets the current generation`, an operation that prepared a
successor died before settling it and its intent still fences the owner. Read
the owner with `execution.status` and `params.issue` / `params.spec`: the
fence is listed under `blocking_prepared_transactions` and
`recommended_recovery` names `execution.release_prepared`. Release it with
that operation, the same owner parameter, a non-empty `params.reason`, and
`params.operation_id` for the exact transaction the diagnosis reported; then
launch. It is owner-addressed rather than session-bound, so a PM agent that
cannot reach the GUI can clear the fence.

## Mode detection

1. If the invocation includes `#N` or an Issue URL, read the Issue with JSON
   operations `issue.view`, `issue.comments`, and `issue.linked_prs`.
2. If the Issue has a case-insensitive `gwt-spec` label, use design-gated mode.
3. If the Issue has no `gwt-spec` label, use direct mode.
4. If no owner Issue exists, use standalone mode.

## design-gated mode

Use this for gwt-spec tagged Issues.

1. Read SPEC sections with JSON operations `issue.spec.read` or
   `issue.spec.section`.
2. If `plan` or `tasks` is missing or incomplete, stop before production edits
   and route to `gwt-plan-spec`.
3. Run the Board active-claim preflight for the owner before choosing a task
   slice.
4. Select the next incomplete task in dependency order.
5. Execute strict TDD: write the RED test first, confirm it fails for the
   expected reason, implement the minimum GREEN change, then refactor.
6. Update the owner Issue tasks section through `issue.spec.edit` as tasks are
   completed.

## direct mode

Use this for non-gwt-spec Issues.

1. Establish the issue facts from JSON operations `issue.view`,
   `issue.comments`, and `issue.linked_prs`.
2. For bugs, prove the root cause before editing production code. Do not
   guess-fix.
3. Classify `Spec Status`: `ALIGNED`, `IMPLEMENTATION-GAP`, `SPEC-GAP`, or
   `SPEC-AMBIGUOUS`.
4. If the intended behavior needs design ownership first, route to
   `gwt-discussion` or `gwt-register-issue`; otherwise continue in direct mode.
5. Execute strict TDD with the same Red-Green-Refactor loop.
6. After verification and PR handoff, post the durable closure comment with
   root cause, changed files, verification evidence, PR link, and remaining
   work if any.

## standalone mode

Use this when the user provides an approved implementation task without an
owner Issue.

1. Capture the task description, acceptance criteria, and target files.
2. Identify the narrowest test location from the existing codebase.
3. Execute strict TDD with the same Red-Green-Refactor loop.
4. Do not create Issues or SPECs unless the work proves to need durable design
   ownership.

## Verification and PR gate

Phase 3 delegates to `gwt-verify --mode full`. Record the selected commands and
results in the evidence bundle. In an `interactive` launch, UI-affecting work
requires a concrete user verification handoff and a `User Verification Result`.

Read the launch route from the launch record, not from the environment:
`execution.status` reports `launch_route: autonomous | manual`. The legacy
`GWT_AUTONOMOUS_EXECUTION` marker still means autonomous when present, but its
absence proves nothing — it is written only when the project opted into
unattended mode, so monitor launches used to misread themselves as human-driven
and stall (#3777, #3697, #4217).

In an `autonomous` launch, skip the user handoff and record
`User Verification Result: n/a (autonomous)`. Do not ask for visual confirmation
or send a verification URL. Cover any UI surface with the agent's own automated
headed E2E, recorded separately as `Agent Visual Check: pass` (`n/a (no UI
surface)` otherwise). Never turn the agent's check into human `confirmed`.

Run the full matrix through `verify.run`. For UI work, select its Playwright
commands with `params.headed_e2e_commands`; each entry must exactly match an
entry in `params.commands`. gwtd adds `--headed` and its embedded reporter and
records actual Chromium results for both dark and light themes. Use
`browser-check` for an isolated checkout instance; the E2E tests must assert the
changed behavior and zero console/page errors. An autonomous UI Ready handoff
requires those measured passing results in the same fresh verification record.

PR work goes through `gwt-manage-pr`. After automated verification and all
other Ready PR Gate conditions pass, create a Ready PR (or call `pr.ready` for
an existing Draft), then follow the existing CI auto-merge path until the PR is
merged. Do not stop at Draft creation. Automated test / headed E2E / CI failures,
known blockers, and other Ready Gate failures still require repair.

The legacy `deferred (autonomous execution)` value remains compatible on
existing autonomous PRs: fresh passing evidence permits Ready without rewriting
the PR body or obtaining human confirmation. Manual launch verification stays
unchanged.

**Never call `execution.blocked` merely because an autonomous launch has no
human visual confirmation.** It is a terminal outcome, not a pause. Continue
through verification, Ready PR, and CI auto-merge; settle the execution after
the scoped delivery is complete.

## Canonical verification admission

Only canonical `verify.run` acquires the host-wide lease, in-process for
its own run. Initial `cargo build -p gwt --bin gwtd`, ordinary `cargo test`,
`cargo clippy`, `cargo build`, coverage, direct headed browser checks, and
pre-push checks do not require a verification lease. Run the RED / GREEN /
refactor commands directly.

For canonical evidence use `verify.plan` then `verify.run`. Manual
`verify.lease.acquire`, `verify.lease.hold`, and `verify.lease.extend` are
retired and return an error without acquiring or reserving a lease.
`verify.run` manages admission and waits up to `params.max_wait_secs`
(default 300, hard cap 1500). A `deferred` response means no verification
record was written: inspect the holder with `verify.lease.status` and
retry when the contention is resolved. There is no manual acquire loop or
fixed retry schedule. `verify.lease.release` remains available to drain a
legacy holder.

## Legacy aliases

During the transition, `$gwt-build-spec SPEC-N` and `$gwt-fix-issue #N` are
accepted only as aliases. Continue as `$gwt-execute #N` and use the mode rules
above. Do not split behavior by SPEC versus Issue once the owner number is
known.
