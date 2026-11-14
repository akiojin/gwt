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
- already delivered with nothing to produce: JSON operation
  `execution.no_action` with a non-empty `params.reason`. No Action is a
  successful *non-delivery*: as an outcome it is neither Completed nor Blocked,
  so it never claims the work shipped and it never files a blocker against an
  owner that has nothing wrong with it. The trusted audit is what records that
  outcome; the execution record itself settles terminally, because a gwtd that
  predates `execution.no_action` knows only the record's statuses and would
  otherwise keep blocking Stop on a correctly settled execution.

`execution.no_action` exists for exactly one situation: a producing generation
was materialized for a **delivered owner** — a closed owner whose whole source
state the configured base already contains — so there is no source work to
verify, commit, push, or hand to a PR. It proves that zero source surface
itself and refuses otherwise, so it cannot be used to skip real work:

- it refuses when the worktree holds any source the base does not contain, when
  the session does not hold the record, when the record was edited outside the
  canonical operations, when the execution is already terminal, and when the
  source surface cannot be proven at all;
- every refusal changes nothing — no execution record, Work, Session, Git, or
  obligation byte moves;
- on success it writes one machine-local integrity-hashed audit that preserves
  the predecessor record byte-identically, settles this action's own
  obligations, and settles the execution so that every reader — including a
  gwtd that predates `execution.no_action` — sees a settled execution rather
  than an Active one. It commits nothing, pushes nothing, requires no
  verification record, and creates or mutates no PR.

A delivered owner is not a blocker: never reach for `execution.blocked` because
an owner turned out to be already shipped. Conversely, never reach for
`execution.no_action` to escape work that exists — it will refuse, and the
refusal names the source it found.

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

For a terminal execution whose local verification admission is unavailable,
do not chase `pr.create` → `verify.run` → `execution.reopen` in a loop or
create a Draft PR. The narrowly scoped exit is `pr.create_from_ci` with
`base`, `head`, `head_sha`, `title`, `body`, and optional `labels`. gwtd reads
the repository's current mandatory GitHub checks for that exact pushed head,
requires every check to pass, and rechecks the refs and policy at dispatch.
Only the exact owning durable Session or the registered repository PM may
use it. The PM nominates the producing head explicitly instead of verifying
the PM worktree. This creates a Ready PR and records a CI delivery audit;
it never reopens the execution or rewrites local verification as PASS.
Permission and UI verification gates remain in force. A moved head, missing
or failed CI, unreadable policy, or missing measured UI evidence refuses the
operation. Keep the existing auto-merge delivery workflow after creation.

When a CLOSED owner is already delivered and its execution is terminal,
use `execution.settle_delivered` with a non-empty `reason` to settle this
Session's remaining obligations, including `pr`, in one operation. gwtd
requires a fresh CLOSED read, zero source surface, and live upstream proof
that HEAD is pushed. It preserves terminal execution, Session and verification
history, takes no verification lease, and claims neither a new PR nor PASS.
An open owner or any unpushed/remaining/unprovable source refuses this exit.

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
first with `execution.status` — its `available_recoveries` names only the
operations the current session can actually run (`verify.plan` / `verify.run`
are listed only when this session holds verification authority). When the
record is terminal and `recovery_hint` is `fresh_launch_required`, nothing in
this session can recover it: stop and use a fresh linked-owner launch.

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
