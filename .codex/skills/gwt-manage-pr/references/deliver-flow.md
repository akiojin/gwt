# PR Deliver Flow (drive-to-merge, detailed)

Deliver mode drives a verified, Ready PR all the way to a merged state. It
**composes** the Fix flow (`references/fix-flow.md`) for blocker resolution and
adds three things on top: auto-merge enablement, a merged-state watch loop
modeled on the `/release` post-merge monitor, and a hard gate that refuses to
arm auto-merge on unverified work.

Deliver does not reimplement CI / review / thread / conflict handling. Every
blocker is resolved through the existing Fix Implementation and Comment
Response steps. Deliver only adds "make it merge, then watch until it does."

## Core invariant: auto-merge is only ever armed against a clear, gated snapshot

Auto-merge is destructive — once armed, GitHub merges the PR the instant its
required checks pass, with no further human step. To keep that safe, Deliver
holds a single invariant:

> Auto-merge may be armed **only** when the PR is fully clear (no blocking CI,
> no conflict/BEHIND, no unresolved thread, no open CHANGES_REQUESTED) **and**
> the Hard PR Gate is satisfied. Before **any** code-changing push, auto-merge
> is **disabled** first; after the push it is **re-gated** and only then
> **re-armed**.

This makes GitHub merge only a snapshot that passed `gwt-verify --mode pre-pr`
and the user verification gate. It inherits the skill's own rule (see SKILL.md
"Ready PR Gate"): *every code-changing re-push to a Ready/non-draft PR must be
preceded by a fresh `gwt-verify --mode pre-pr` PASS with a satisfied
`User Verification Result`*. Deliver does not override that rule — it enforces
it on every drive iteration.

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

## Step 1: Hard PR Gate (mandatory before enabling auto-merge)

Do **not** enable auto-merge until the Ready PR Gate passes for the PR scope.
Deliver applies a stricter gate than Create/Fix because auto-merge removes the
last human checkpoint:

- `gwt-verify --mode pre-pr` returns `Overall: PASS`.
- `User Verification Result` is `confirmed` (user visually verified) or `n/a`
  (the change has no user-visible surface, so visual verification does not
  apply).
- The PR is a releaseable slice with no known blockers in its scope.
- The PR is **not** a Draft (auto-merge cannot be armed on a Draft PR, and a
  Draft is by definition unfinished).

Refuse to arm auto-merge when any of these hold:

- `User Verification Result` is `pending` (the verification handoff never
  completed) or `rejected(<reason>)` (the user declined).
- `User Verification Result` is `skipped(<reason>)`. A skip is acceptable for
  *creating* a Draft/Ready PR per the shared Ready PR Gate, but it is **not**
  sufficient to merge unattended — Deliver requires `confirmed` or `n/a`.
- `gwt-verify --mode pre-pr` returns `Overall: FAIL` or `failed: tooling-missing`.

On gate failure, stop. Do not run `gh pr merge --auto`. Route the failure for
repair (back to the TDD loop, `gwt-verify`, or `gwt-discussion`) and report
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
  field in the output. `gh pr view` is hook-blocked, so always read through
  `pr.view`.

## Step 3: Resolve all blockers before arming (Fix flow)

Inspect with Fix mode `--mode all` and resolve **every** BLOCKING item through
the existing Fix Implementation / Comment Response flow in `fix-flow.md` —
**before** arming auto-merge:

- `CI-FAILURE` -> fix code, commit, push.
- `CHANGE-REQUEST` / `REVIEW-COMMENT` / `UNRESOLVED-THREAD` -> apply the change,
  reply to and resolve the thread (`pr.review_threads.reply_and_resolve`), then
  post the reviewer summary (`pr.comment`).
- `CONFLICT` / `BRANCH-BEHIND` -> `git fetch origin <base> && git merge
  origin/<base> && git push` (never rebase); resolve conflicts per fix-flow.

Per the Core invariant, **every code-changing push here re-runs the Hard PR
Gate (Step 1) before the drive proceeds**. Do not arm auto-merge while any
blocking item remains.

## Step 4: Select the merge method (project-agnostic)

Auto-merge requires a merge method allowed by the target repository. Do **not**
hardcode `--squash`; managed skills run against many repositories with different
policies. Query the repository's allowed methods and default:

```bash
gh repo view --json mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed,viewerDefaultMergeMethod
```

- Use `viewerDefaultMergeMethod` when it is allowed (`MERGE` -> `--merge`,
  `SQUASH` -> `--squash`, `REBASE` -> `--rebase`).
- If the default is disallowed, pick the single allowed method.
- Never change branch protection or merge policy to force a method. If no method
  is allowed, report the policy blocker and stop.

`gh repo view` is not hook-blocked, so this query is allowed.

## Step 5: Arm auto-merge on a clear snapshot

Only once Step 3 leaves the PR fully clear (no blocking CI, no conflict/BEHIND,
no unresolved thread, no open CHANGES_REQUESTED) and the only thing left is
required checks still running, arm auto-merge with the selected method
(transport exception: there is no `pr.merge` JSON operation, and `gh pr merge`
is an allowed command — only `gh pr view/create/edit/ready/draft/comment/
checks/reviews/review-threads` are blocked):

```bash
gh pr merge --auto --merge <number>   # method from Step 4
```

GitHub merges the PR once all **required** status checks pass.

### Branch-protection dependency (read before relying on `--auto`)

`gh pr merge --auto` waits only for **required status checks**. It does **not**
block on unresolved review threads or `CHANGES_REQUESTED` reviews unless the
repository's branch protection enforces *require conversation resolution* and/or
*required approvals*. On repositories that do **not** enforce those rules, a
review filed after auto-merge is armed can be raced by server-side merge before
the synchronous Fix loop processes it.

- Preferred on protected repos (conversation-resolution + approvals enforced):
  arm `--auto` from a clear snapshot as above.
- On repos **without** those protections: prefer the poll-then-merge path
  (below) instead of `--auto`, so the agent — not the server — performs the
  merge after re-confirming a clear state. Keep auto-merge disabled while any
  unresolved thread or open `CHANGES_REQUESTED` exists.

### Poll-then-merge (no `--auto`)

Use this when repository auto-merge is disabled, or as the preferred path on
unprotected repos. Run the watch loop (Step 6); when `pr.view` reports
`mergeable: MERGEABLE` / `merge_state: CLEAN` with required checks green and no
unresolved thread, **re-read `pr.view` immediately before merging** and require
that clear state at that instant (guard against a base advance or check flip
between read and merge), then merge directly:

```bash
gh pr merge --merge <number>
```

The Hard PR Gate (Step 1) and the per-push re-gate (Core invariant) still apply
— a direct merge is only allowed on a verified, gated snapshot.

## Step 6: Watch for merge, re-gate on any new blocker

Poll JSON operation `pr.view` to observe progress toward merge:

- Re-read `pr.view` at ~30s intervals while the PR is unmerged.
- The PR is delivered when `pr.view` shows the `[MERGED]` state (GitHub's
  `merged_at` is set). That is the only completion signal — "auto-merge enabled"
  is not "merged."
- If a **new** blocker appears before merge — base advanced (`merge_state:
  BEHIND`), a required check fails, a new review thread, a new
  `CHANGES_REQUESTED` — first **disable auto-merge**, then resolve it:

  ```bash
  gh pr merge --disable-auto <number>
  ```

  Resolve the blocker through the Fix flow (Step 3). For any code-changing push,
  **re-run the Hard PR Gate (Step 1)**, then **re-arm** auto-merge (Step 5) only
  after the PR is clear and gated again. Never leave auto-merge armed across a
  code-changing push.
- This poll is **bounded**. If required checks stay pending/queued with no
  progress for ~20 polls (~10 minutes) and nothing is failing, post JSON
  operation `board.post` with `params.kind:"blocked"`, the PR number, the
  pending check names, and a resume instruction, then stop instead of sleeping
  indefinitely. For longer CI, arm a completion goal (Step 9) instead of
  long-polling.

## Step 7: Re-run transient CI failures (narrowed for Deliver)

When `pr.checks` / `actions.logs` show a **failed** required check, classify it
before deciding to re-run vs fix. Deliver narrows the `/release` step 13.3
classifier: in Deliver the agent is merging arbitrary PR code, so a real
code-induced hang or a flaky test must not be re-run into a silent merge.

- **Transient / infrastructure** -> re-run the failed jobs. Treat a signal as
  transient **only when it originates from job setup, network, registry, or
  runner provisioning** (not from a test/build step): `unable to update
  registry`, `download of ... failed`, `curl failed`, `Error in the HTTP2
  framing layer`, `TLS connect error`, `429` / `rate limit`, `503`, runner
  provisioning failures, or a network `Connection reset` / `timed out` during
  setup/checkout.

  ```bash
  gh run rerun <run-id> --failed
  ```

  `gh run rerun` and `gh run list` are allowed (only `gh run view` is blocked;
  read logs through `actions.logs` / `actions.job_logs`, which require the run
  to be completed). Re-run a given run at most 3 times.

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
  PR #<number> to merged: keep the verified snapshot armed, disable+re-gate
  auto-merge on every code-changing push, re-run only infrastructure-transient
  CI failures, and finish when `pr.view` shows `[MERGED]` (`merged_at` set).
  Stop and report on non-transient failures or a blocker that survives the Loop
  Safety Guard. Cap at 60 minutes / 30 turns."
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
  verification that gated auto-merge (including re-gates performed), and the
  merge method used.
- Note any transient re-runs performed and their count.
- If the loop stopped before merge (gate failure, Loop Safety Guard, non-
  transient blocker, bounded-poll handoff, Draft-only Create fallback), report
  the exact stop reason and the remaining blocker instead of claiming delivery.

## Command surface (allowed vs blocked)

- **Allowed bash**: `gh pr merge` (`--auto`, `--disable-auto`, and direct),
  `gh repo view`, `gh run list`, `gh run rerun`,
  `gh release view`.
- **Blocked bash** (use the JSON operation instead): `gh pr view` ->
  `pr.view`; `gh pr ready` -> `pr.ready`; `gh pr ready --undo` -> `pr.draft`;
  `gh pr checks` -> `pr.checks`; `gh pr comment` -> `pr.comment`;
  `gh pr review-threads` -> `pr.review_threads`; `gh run view` ->
  `actions.logs` / `actions.job_logs`.
- There is **no** `pr.merge` JSON operation; auto-merge enable/disable is the
  documented `gh pr merge` transport exception. Draft <-> Ready transitions go
  through the JSON operations `pr.ready` / `pr.draft` (Issue #3201).

## Anti-patterns (prohibited)

| Prohibited | Required alternative |
|---|---|
| Enable auto-merge on `pending` / `skipped` verification | Gate on `confirmed` or `n/a` (Step 1) |
| Keep auto-merge armed across a code-changing push | Disable, re-gate, re-arm per push (Core invariant / Step 6) |
| Arm `--auto` while a blocker still exists | Resolve all blockers first, arm from a clear snapshot (Step 5) |
| Rely on `--auto` to block on threads on an unprotected repo | Prefer poll-then-merge; arm `--auto` only on protected repos (Step 5) |
| Auto-route into Deliver without an explicit request | Deliver is opt-in only |
| Report "delivered" after enabling auto-merge | Report Delivered only when `pr.view` shows `[MERGED]` (`merged_at` set) |
| Blindly `gh run rerun` a test/build timeout or compile/test failure | Classify infra-transient vs code (Step 7) |
| Hardcode `--squash` | Use the repo's `viewerDefaultMergeMethod` (Step 4) |
| Poll `merged_at` forever | Bounded poll + `board.post` blocked handoff or goal (Step 6/9) |
