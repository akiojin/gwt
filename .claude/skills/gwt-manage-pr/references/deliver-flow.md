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
- `pr.draft` explicitly holds a merge when the owner requests it: a Draft PR
  cannot be merged. Resume with `pr.ready` after the Ready PR Gate passes.
  Routine pushes do not require this hold.

## Preserve enabled auto-merge

Keep auto-merge enabled across routine pushes and base synchronization. Before
every code-changing push, run fresh `gwt-verify --mode pre-pr` and satisfy the
User Verification Result gate. CI checks the pushed snapshot before merging.

For a BEHIND PR, use JSON operation `pr.update_branch`. If the PM owns base
synchronization, post the delivery handoff on the Board and let the PM perform
it; do not race another updater. Resolve actual content conflicts through Fix.
Do not require a Draft/Ready cycle or an unavailable merge operation merely
because code or the base branch changed.

## Entry contract

- For `execution.status` reporting `launch_route: autonomous`, continue
  through a verified Ready PR and the existing CI auto-merge path until merged.
  Human visual confirmation is not a prerequisite.
- For manual launches, Deliver runs only on explicit user intent: "deliver",
  "drive to merge", "merge it", "land the PR", "ship it", or an equivalent
  direct request. Manual auto-detection does not select Deliver.
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
  the handoff is waived). UI work requires `Agent Visual Check: pass` and
  actual passing headed Chromium results for dark and light themes in the same
  fresh `verify.run` record, selected with `params.headed_e2e_commands`.
- Existing autonomous PRs may retain the legacy
  `deferred (autonomous execution)` body value and pass with fresh evidence;
  no body rewrite or human confirmation is required.
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
- `gwt-verify --mode pre-pr` returns `Overall: FAIL` or `failed: tooling-missing`.

On gate failure, do not push unverified code or hand new work over through
`pr.ready`. Route the failure for repair (back to the TDD loop, `gwt-verify`,
or `gwt-discussion`) and report
`NO ACTION` with the failing gate item. Never downgrade a `pending` result to
`skipped` to get past the gate.

Re-run this gate before every code-changing push during the drive loop.

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
- `CONFLICT` -> fetch the base, merge it locally, resolve conflicts per
  fix-flow, verify, then push (never rebase).
- `BRANCH-BEHIND` -> JSON operation `pr.update_branch`, or a Board handoff
  when the PM owns base synchronization.

**Every code-changing push re-runs the Hard PR Gate (Step 1) first.** Keep
auto-merge enabled while resolving blockers; CI remains the delivery gate.

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

Merge automation follows the repository's required checks and review rules.
Resolve review blockers through Fix and preserve the existing protection
settings and enabled auto-merge. An explicit owner-requested hold is the
separate `pr.draft` operation described above.

## Step 6: Watch for merge, re-gate on any new blocker

Poll JSON operation `pr.view` to observe progress toward merge:

- Re-read `pr.view` at ~30s intervals while the PR is unmerged.
- The PR is delivered when `pr.view` shows the `[MERGED]` state (GitHub's
  `merged_at` is set). That is the only completion signal — "handed to merge
  automation" is not "merged."
- If a **new** blocker appears before merge — base advanced (`merge_state:
  BEHIND`), a required check fails, a new review thread, a new
  `CHANGES_REQUESTED` — resolve it through the Fix flow (Step 3) while keeping
  auto-merge enabled. For any code-changing push, **re-run the Hard PR Gate
  (Step 1)** first. Use `pr.update_branch` or the PM handoff for base sync.
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
  PR #<number> to merged: keep auto-merge enabled and re-gate before every
  code-changing push, synchronize the base through its assigned owner, re-run only
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
| Enable auto-merge on `pending` / `skipped` verification | Gate on `confirmed` or `n/a` (Step 1) |
| Require disabling auto-merge before routine pushes | Keep auto-merge enabled and verify before each code-changing push |
| Arm auto-merge while a blocker still exists | Resolve all blockers first, arm from a clear snapshot (Step 5) |
| Leave review blockers unresolved | Resolve threads through Fix and preserve repository protection settings |
| Auto-route a manual launch into Deliver without an explicit request | Manual Deliver is opt-in; autonomous delivery follows its launch contract |

| Report "delivered" after enabling auto-merge | Report Delivered only when `pr.view` shows `[MERGED]` (`merged_at` set) |
| Blindly rerun a test/build timeout or compile/test failure | Classify infra-transient vs code, then use `actions.rerun` only for infrastructure failures (Step 7) |
| Poll `merged_at` forever | Bounded poll + `board.post` blocked handoff or goal (Step 6/9) |
