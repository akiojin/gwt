# PR Deliver Flow (drive-to-merge, detailed)

Deliver mode drives a verified, Ready PR all the way to a merged state. It
**composes** the Fix flow (`references/fix-flow.md`) for blocker resolution and
adds three things on top: a hand-over to the repository's merge automation, a
merged-state watch loop modeled on the `/release` post-merge monitor, and a
hard gate that refuses to hand unverified work to that automation.

Deliver does not reimplement CI / review / thread / conflict handling. Every
blocker is resolved through the existing Fix Implementation and Comment
Response steps. Deliver only adds "make it merge, then watch until it does."

## Who merges: gwtd has no merge operation

gwtd has no merge operation, and the agent never merges a PR itself. The merge
is performed by the repository's merge automation — GitHub auto-merge enabled
on the PR, or a repository workflow that merges non-Draft PRs once their checks
pass — or by a human. Deliver works through two JSON operations:

- `pr.ready` hands the PR to that automation (arms the merge).
- `pr.draft` **holds the merge**: a Draft PR cannot be merged, whichever
  automation would otherwise merge it.

## Core invariant: only a clear, gated snapshot is ever mergeable

Merge automation is destructive — once a PR is handed over, it merges the
instant its required checks pass, with no further human step. To keep that
safe, Deliver holds a single invariant:

> The PR may sit non-Draft under merge automation **only** when it is fully
> clear (no blocking CI, no conflict/BEHIND, no unresolved thread, no open
> CHANGES_REQUESTED) **and** the Hard PR Gate is satisfied. Before **any**
> code-changing push, the merge is **held** first; after the push it is
> **re-gated** and only then **re-armed**.

This makes the automation merge only a snapshot that passed
`gwt-verify --mode pre-pr` and the user verification gate. It inherits the
skill's own rule (see SKILL.md "Ready PR Gate"): *every code-changing re-push
to a Ready/non-draft PR must be preceded by a fresh `gwt-verify --mode pre-pr`
PASS with a satisfied `User Verification Result`*. Deliver does not override
that rule — it enforces it on every drive iteration.

## Entry contract (opt-in only)

- Deliver runs **only** on explicit user intent: "deliver", "drive to merge",
  "merge it", "land the PR", "ship it", or an equivalent direct request to take
  the PR to merged.
- Deliver is **never auto-routed**. The Mode Auto-Detection 2x2 matrix must not
  select Deliver on its own.
- If no open PR exists for the current branch, fall back to Create mode first
  (Ready PR Gate applies). If Create can only produce a **Draft** (the Ready PR
  Gate is not satisfied), **stop with NO ACTION** — do not enter the drive loop
  on a Draft. Only drive a PR that Create produced as Ready.

## Step 1: Hard PR Gate (mandatory before handing the PR to merge automation)

Do **not** hand the PR to merge automation until the Ready PR Gate passes for
the PR scope. Deliver applies a stricter gate than Create/Fix because merge
automation removes the last human checkpoint:

- `gwt-verify --mode pre-pr` returns `Overall: PASS`.
- `User Verification Result` is `confirmed` (user visually verified), `n/a`
  (the change has no user-visible surface, so visual verification does not
  apply), or `n/a (autonomous)` (an unattended gwt Issue Monitor launch, where
  the handoff is waived and `Agent Visual Check: pass` carries any UI surface).
- The PR is a releaseable slice with no known blockers in its scope.

Refuse to hand the PR over when any of these hold:

- `User Verification Result` is `pending` (the verification handoff never
  completed) or `rejected(<reason>)` (the user declined).
- `User Verification Result` is `skipped(<reason>)`. A skip is acceptable for
  *creating* a Draft/Ready PR per the shared Ready PR Gate, but it is **not**
  sufficient to merge unattended — Deliver requires `confirmed`, `n/a`, or
  `n/a (autonomous)`. Never relabel a `skipped(<reason>)` as
  `n/a (autonomous)`: the waiver comes from the launch route recorded on the
  Session (`execution.status` → `launch_route: autonomous`; the legacy
  `GWT_AUTONOMOUS_EXECUTION` marker still counts when present, but its absence
  proves nothing), not from the agent finding the check inconvenient.
- `User Verification Result` is `deferred (autonomous execution)`. That value
  says the owner's visual check has not happened yet, so the PR stays Draft by
  design and gwt refuses to mark it Ready. It reaches merge automation only
  after the owner sweeps it and the result becomes `confirmed`.
- `gwt-verify --mode pre-pr` returns `Overall: FAIL` or `failed: tooling-missing`.

On gate failure, stop. Do not hand the PR over through `pr.ready`; if it is
already non-Draft, hold it through `pr.draft`. Route the failure for repair
(back to the TDD loop, `gwt-verify`, or `gwt-discussion`) and report
`NO ACTION` with the failing gate item. Never downgrade a `pending` result to
`skipped` to get past the gate.

This same gate is re-run after every code-changing push during the drive loop
(see the Core invariant and Step 6).

## Step 2: Resolve the PR

- Resolve the current-branch PR through JSON operation `pr.current`, or use the
  user-supplied PR number / URL.
- Read merge-relevant state through JSON operation `pr.view`. Its output lines
  are: `#<n> [<state>] <title>` (state `OPEN` / `CLOSED` / `MERGED`), `url:`,
  `ci:`, `mergeable:`, `merge_state:`, and `review:`. `mergeable:` carries
  GitHub's mergeability (`MERGEABLE` / `CONFLICTING` / `UNKNOWN`) and
  `merge_state:` the merge-state status (`CLEAN` / `BEHIND` / `BLOCKED` /
  `DIRTY` / `UNKNOWN`). The merged signal is the `[MERGED]` state bracket
  (GitHub's `merged_at` being set); there is no literal `merged_at` / `isDraft`
  field in the output. Read-only `gh pr view` is also allowed; this flow uses
  `pr.view` for its workflow lifecycle context and stable completion signal.

## Step 3: Resolve all blockers before handing over (Fix flow)

Inspect with Fix mode `--mode all` and resolve **every** BLOCKING item through
the existing Fix Implementation / Comment Response flow in `fix-flow.md` —
**before** handing the PR to merge automation:

- `CI-FAILURE` -> fix code, commit, push.
- `CHANGE-REQUEST` / `REVIEW-COMMENT` / `UNRESOLVED-THREAD` -> apply the change,
  reply to and resolve the thread (`pr.review_threads.reply_and_resolve`), then
  post the reviewer summary (`pr.comment`).
- `CONFLICT` / `BRANCH-BEHIND` -> `git fetch origin <base> && git merge
  origin/<base> && git push` (never rebase); resolve conflicts per fix-flow.

Per the Core invariant, **every code-changing push here re-runs the Hard PR
Gate (Step 1) before the drive proceeds**. Keep the merge held while any
blocking item remains.

## Step 4: Confirm the PR has merge automation

gwtd cannot merge, so Deliver only reaches `[MERGED]` when something else
merges. Confirm that one of these applies to the PR:

- GitHub auto-merge is enabled on it (read-only `gh pr view --json
  autoMergeRequest`), or
- the repository has a workflow that merges non-Draft PRs once their checks
  pass (read its definition under `.github/workflows/`).

If neither applies, the merge needs a human. Report the clear, gated PR with
`Action: NO ACTION` and the reason "no merge automation; a human must merge",
then stop. Never change branch protection or repository settings to create
automation.

These read-only queries are allowed and recorded on the shared GitHub budget
ledger.

## Step 5: Hand a clear snapshot to merge automation

Only once Step 3 leaves the PR fully clear (no blocking CI, no conflict/BEHIND,
no unresolved thread, no open CHANGES_REQUESTED) and the only thing left is
required checks still running, hand it over: if the PR is Draft, use JSON
operation `pr.ready`. The automation merges the PR once all **required**
status checks pass.

### Branch-protection dependency (read before relying on merge automation)

Merge automation usually waits only for **required status checks**. It does
**not** block on unresolved review threads or `CHANGES_REQUESTED` reviews
unless the repository's branch protection enforces *require conversation
resolution* and/or *required approvals*. On repositories that do **not**
enforce those rules, a review filed after hand-over can be raced by the
server-side merge before the synchronous Fix loop processes it.

- Preferred on protected repos (conversation-resolution + approvals enforced):
  hand over from a clear snapshot as above.
- On repos **without** those protections: keep the PR Draft through the watch
  loop (Step 6), and hand it over only when `pr.view` reports
  `mergeable: MERGEABLE` / `merge_state: CLEAN` with required checks green and
  no unresolved thread. **Re-read `pr.view` immediately before `pr.ready`** and
  require that clear state at that instant (guard against a base advance or
  check flip between read and hand-over).

The Hard PR Gate (Step 1) and the per-push re-gate (Core invariant) still apply
— only a verified, gated snapshot is handed over.

## Step 6: Watch for merge, re-gate on any new blocker

Poll JSON operation `pr.view` to observe progress toward merge:

- Re-read `pr.view` at ~30s intervals while the PR is unmerged.
- The PR is delivered when `pr.view` shows the `[MERGED]` state (GitHub's
  `merged_at` is set). That is the only completion signal — "handed to merge
  automation" is not "merged."
- If a **new** blocker appears before merge — base advanced (`merge_state:
  BEHIND`), a required check fails, a new review thread, a new
  `CHANGES_REQUESTED` — first **hold the merge** through JSON operation
  `pr.draft`, then resolve it.

  Resolve the blocker through the Fix flow (Step 3). For any code-changing push,
  **re-run the Hard PR Gate (Step 1)**, then **re-arm** by handing the PR back
  through `pr.ready` (Step 5) only after the PR is clear and gated again. Never
  leave a PR mergeable across a code-changing push.
- This poll is **bounded**. If required checks stay pending/queued with no
  progress for ~20 polls (~10 minutes) and nothing is failing, post JSON
  operation `board.post` with `params.kind:"blocked"`, the PR number, the
  pending check names, and a resume instruction, then stop instead of sleeping
  indefinitely. For longer CI, arm a completion goal (Step 9) instead of
  long-polling.

## Step 7: Re-run transient CI failures (narrowed for Deliver)

When `pr.checks` / `actions.logs` show a **failed** required check, classify it
before deciding to re-run vs fix. Deliver narrows the `/release` step 13.3
classifier: in Deliver the agent is driving arbitrary PR code to merge, so a
real code-induced hang or a flaky test must not be re-run into a silent merge.

- **Transient / infrastructure** -> re-run the failed jobs. Treat a signal as
  transient **only when it originates from job setup, network, registry, or
  runner provisioning** (not from a test/build step): `unable to update
  registry`, `download of ... failed`, `curl failed`, `Error in the HTTP2
  framing layer`, `TLS connect error`, `429` / `rate limit`, `503`, runner
  provisioning failures, or a network `Connection reset` / `timed out` during
  setup/checkout.

  ```text
  {"schema_version":1,"operation":"actions.rerun",
   "params":{"run_id":<run-id>,"failed_only":true}}
  ```

  Prefer `{"job_id":<job-id>}` — the id `pr.checks` already reports — so a
  single flaky check does not re-run every job in the run. `actions.rerun`
  refuses a run/job the current repository does not own. Read logs through
  `actions.logs` / `actions.job_logs`, which require the run to be
  completed. Re-run a given run at most 3 times.

- **Not auto-transient in Deliver** -> a `timed out` reported **by a test or
  build step**, a compile error `error[E####]`, test `FAILED` / `panicked`,
  clippy, lint, signing, or a missing secret. These are code/config problems.
  Do **not** blindly re-run. Return to Step 3 and fix, or report and stop if it
  needs a product decision.
- A check that **fails and then passes on re-run without any code change** is a
  possible flaky or real timing bug. Report it; do not let it silently merge.

## Step 8: Loop Safety Guard

The same blocker surviving 3 consecutive drive iterations stops the loop:

- "Same blocker" means the same CI check name, the same unresolved thread, or
  the same conflict failing 3 iterations in a row.
- On the 3rd consecutive failure: report which blocker, what was attempted each
  iteration, and what keeps failing, then ask the user **continue** / **abort**
  / **change approach**. Proceed only after an explicit decision.
- Different blockers failing in different iterations do **not** trip the guard —
  that is normal progress.

## Step 9: Arm a completion goal (optional, SPEC-3050)

Driving to merge can span CI runs longer than one turn. Optionally arm a "PR
merged" completion goal so monitoring survives turn boundaries, using the same
goal-start contract as `/release` step 5.4:

- **Codex** (goals enabled): call `create_goal` with an objective like "Drive
  PR #<number> to merged: keep only the verified snapshot mergeable, hold the
  merge with `pr.draft` and re-gate on every code-changing push, re-run only
  infrastructure-transient CI failures, and finish when `pr.view` shows
  `[MERGED]` (`merged_at` set). Stop and report on non-transient failures or a
  blocker that survives the Loop Safety Guard. Cap at 60 minutes / 30 turns."
- **Claude Code** (v2.1.139+): self-`/goal` is not invokable directly; queue it
  to the current pane via JSON operation `pane.send` with text `/goal <the same
  condition>` (self-only; targets the `GWT_SESSION_ID` pane).

Arming the goal is best effort. If it cannot be armed (older runtime, goals
disabled, `pane.send` failure), print the `/goal <condition>` line for the user
to run manually and continue. The goal is a "do not stop early" guarantee, not a
replacement for the in-loop poll in Step 6 — run the poll either way.

## Step 10: Final report (Delivered)

Report using the skill's Final Report Contract. When the PR reached the
`[MERGED]` state, the `Action` is `Delivered`:

- `Action: Delivered`, PR number + URL, base <- head.
- `PR Update Summary`: commits, what merged, related Issue/SPEC, the
  verification that gated the hand-over (including re-gates performed), and
  which merge automation merged it.
- Note any transient re-runs performed and their count.
- If the loop stopped before merge (gate failure, Loop Safety Guard, non-
  transient blocker, bounded-poll handoff, no merge automation, Draft-only
  Create fallback), report the exact stop reason and the remaining blocker
  instead of claiming delivery.

## Command surface (allowed vs blocked)

- **Allowed Bash reads** include `gh pr view/list/checks/diff/status`,
  `gh issue view/list/status`, `gh run view/list/watch`, `gh repo view`,
  `gh release view`, and read-only `gh api` requests. Allowed calls are
  recorded on the shared GitHub budget ledger. Prefer gwtd JSON operations
  such as `pr.list` and `issue.view` for cached data and workflow lifecycle
  context.
- **Mutations require JSON-envelope operations**: `gh pr ready` -> `pr.ready`;
  `gh pr ready --undo` -> `pr.draft`; `gh pr comment` -> `pr.comment`;
  `gh run rerun` -> `actions.rerun`. `gh pr merge` has no gwtd counterpart:
  gwtd has no merge operation, so merging stays with the repository's merge
  automation or a human. Other writes, including API mutations, also require
  the corresponding JSON-envelope operation.
- Read permission does not bypass the Hard PR Gate, Ready PR Gate, bounded
  polling, or verification evidence requirements.

## Anti-patterns (prohibited)

| Prohibited | Required alternative |
|---|---|
| Hand a PR to merge automation on `pending` / `skipped` verification | Gate on `confirmed` or `n/a` (Step 1) |
| Leave a PR mergeable across a code-changing push | Hold with `pr.draft`, re-gate, re-arm with `pr.ready` per push (Core invariant / Step 6) |
| Hand over while a blocker still exists | Resolve all blockers first, hand over a clear snapshot (Step 5) |
| Rely on merge automation to block on threads on an unprotected repo | Keep the PR Draft until CLEAN, then hand over (Step 5) |
| Look for a gwtd merge operation or merge with `gh pr merge` | gwtd has no merge operation; rely on merge automation or a human (Step 4) |
| Auto-route into Deliver without an explicit request | Deliver is opt-in only |
| Report "delivered" after handing the PR over | Report Delivered only when `pr.view` shows `[MERGED]` (`merged_at` set) |
| Blindly rerun a test/build timeout or compile/test failure | Classify infra-transient vs code, then use `actions.rerun` only for infrastructure failures (Step 7) |
| Poll `merged_at` forever | Bounded poll + `board.post` blocked handoff or goal (Step 6/9) |
