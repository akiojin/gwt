---
name: gwt-manage-pr
description: "Use when the user wants to create, inspect, update, or unblock a pull request and expects one visible entrypoint for the PR lifecycle."
---

# gwt-manage-pr — Unified PR Lifecycle Manager

## Overview

Single skill for the full PR lifecycle: create, check status, fix blockers, and deliver (drive to merge). Auto-detects the appropriate mode from current branch PR state, or accepts an explicit mode from the user. Deliver is automatic for autonomous execution after the Ready PR Gate passes; manual launches require an explicit delivery request.

Use the current user's language for decision summaries, blocker reports, and
next-step guidance returned from this workflow.

Read-only `gh` commands are allowed and recorded on the shared GitHub budget
ledger. Prefer gwtd JSON operations such as `pr.list` and `issue.view` when
cached data or workflow lifecycle context is needed. GitHub mutations must
use JSON-envelope operations, including `pr.ready` / `pr.draft` for handing a
PR to merge automation or holding it back, and `actions.rerun` for rerunning
CI. gwtd has no merge operation. All verification and Ready PR gates still
apply. The current
implementation may still use GitHub REST / `gh` internally as transport, while
GraphQL remains the transport for unresolved review threads and thread
reply/resolve.

## Final Report Contract

Every final user-facing report from this workflow MUST state whether a PR was
created, updated, fixed, delivered (driven to merge), or left unchanged. When a
PR was created, pushed to, fixed, or delivered, include a `PR Update Summary`
derived from the actual diff, commit range, and PR body. Do not invent changes
from intent alone.

`PR Update Summary` must include:

- PR number and URL
- base/head branches
- commit count or updated commit range
- 2-5 bullets describing what the PR updates
- related Issue/SPEC numbers, or `None`
- verification and CI/review status

For **check** and **NO ACTION** results, explicitly say no PR update was made.
If an existing PR is only inspected, summarize the current PR status separately
from `PR Update Summary` so the report does not imply new changes were pushed.

Final report shape:

```text
## PR Result
- Action: Created | Updated | Fixed | Delivered | Checked | No Action
- PR: #<number> <url> | None
- Branches: <base> <- <head>

### PR Update Summary
- Commits: <count or range>
- Updates:
  - <what changed>
- Related: #<issue/spec> | None
- Verification: <commands/checks and status>

### Next
- <remaining blocker or "None">
```

Omit `PR Update Summary` only when no PR was created, updated, or fixed; in
that case include `PR Update: none` in the result.

## gwtd resolution

Before executing any `gwtd ...` command from this skill or its references,
resolve `GWT_BIN` first: executable `GWT_BIN_PATH`, then `command -v gwtd`,
then `$GWT_PROJECT_ROOT/target/debug/gwtd` or `./target/debug/gwtd`. Run the
command as `"$GWT_BIN" ...`; if none exists, stop with an actionable
`gwtd not found` error.

## Mode Auto-Detection

On invocation, run Shared Preflight, then route:

1. **Preflight** — always runs first (see [Shared Preflight](#shared-preflight)).
2. **Explicit mode** (user said "check" / "fix" / "create" / "deliver"):
   - "check status" / "PR status" / "is it merged?" → **check** mode
   - "fix CI" / "fix the PR" / "resolve blockers" → **fix** mode
   - "create PR" / "open PR" → **create** mode (with smart skip if open PR exists)
   - "deliver" / "drive to merge" / "merge it" / "land the PR" / "ship it" /
     "watch until merged" → **deliver** mode (Ready PR Gate must be satisfied)
3. **Autonomous execution** — when `execution.status` reports
   `launch_route: autonomous`, use **deliver** after verification and the Ready
   PR Gate pass, continuing through the existing CI auto-merge path until merged.
4. **Auto-detect for manual launches** (no explicit mode) — use the
   commit-count-first decision from Preflight Step 7. Manual auto-detect never
   selects **deliver**; enabling auto-merge requires an explicit user request:
   - open PR + `mergeable: CONFLICTING|DIRTY|BEHIND` → **fix**
   - `N > 0` + no open PR → **create**
   - `N > 0` + open PR + clean merge state → **create** (push-only + post-push fix)
   - `N = 0` + open PR → **fix** (check CI / reviews / conflicts)
   - `N = 0` + no open PR → report **NO ACTION**

## Shared Preflight

Every mode begins with these steps. **The order is intentional — commit
count against the base branch comes before PR state lookup so that the
MERGED shortcut ("PR is done therefore nothing to do") is structurally
impossible.**

1. **Repo + branches:**
   - `git rev-parse --show-toplevel` / `git rev-parse --abbrev-ref HEAD`
   - Base defaults to `develop` unless user specifies.
2. **Branch protection:** Only `develop` may target `main`. Refuse any other branch targeting `main`.
3. **Working tree state:** `git status --porcelain`. If dirty, pause and present options (continue / abort / cleanup). Do not auto-commit/stash.
4. **Fetch:** `git fetch origin`
5. **Commit count against base (mandatory first check):**
   ```bash
   N=$(git rev-list --count "origin/$base..HEAD")
   ```
   > **Baseline ref rule**: ALWAYS compare against `origin/<base>`
   > (the PR target branch, default `origin/develop`). NEVER use
   > `origin/<head>` (the remote tracking branch of the current
   > branch) — that only tells you if commits are pushed, not
   > whether develop has your work. Confusing the two is the root
   > cause of the MERGED-state false NO ACTION bug.
6. **PR lookup + open-PR check:**
   - Prefer JSON operation `pr.current` as the normal read path for the current branch.
   - Treat the literal line `no current pull request` as the canonical no-PR sentinel.
   - If create mode needs lower-level repo/head lookup, treat it as internal transport managed by the toolchain rather than part of the agent-facing workflow.
   - `has_open_pr` = any entry with `state == "open" && merged_at == null`
   - When an open PR exists, inspect `mergeable:`. `CONFLICTING`, `DIRTY`, and `BEHIND` are blocking merge states and immediately upgrade routing to **fix**.
7. **Route using the 2×2 matrix, with merge-state override:**

   | Commits (N) | Open PR? | Action |
   |---|---|---|
   | N > 0 | No | **CREATE** new PR |
   | N > 0 | Yes | **PUSH ONLY** to existing PR → then **FIX** |
   | N = 0 | Yes | **FIX** (CI / reviews / conflicts) |
   | N = 0 | No | **NO ACTION** |

   If the open PR reports `mergeable: CONFLICTING`, `DIRTY`, or `BEHIND`,
   use **FIX** immediately instead of push-only/create. Outside that
   override, PR state (MERGED / CLOSED) is not consulted.

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

In gwt-managed execution worktrees the canonical operations enforce part of
this mechanically (SPEC-3248 P8b): non-draft `pr.create` and `pr.ready`
refuse when the session's Execution Control Record is terminally blocked, or
active without a fresh all-passing Verification Run Record (written by JSON
operation `verify.run`). Run the pre-PR matrix through `verify.run` so the
evidence exists before the Ready handoff; Draft PR creation stays available
mid-work.

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

## Mode: Create

Detailed logic in `references/create-flow.md`.

### Decision Rules

Create mode is entered from the Preflight 2×2 matrix when `N > 0`.

1. **Do not create or switch branches.** Always use the current branch as head.
2. **If open PR exists and is `CONFLICTING` / `DIRTY` / `BEHIND`** → enter Fix mode before any push-only path.
3. **If the work fails the Ready PR Gate** → create/update only as Draft PR, or return NO ACTION if the user explicitly rejects Draft.
4. **If open PR exists and merge state is clean** → push only, return existing PR URL, enter Fix mode. A Ready/non-draft PR still requires the Ready PR Gate.
5. **If no open PR** → create new PR with `params.draft:true` unless the Ready PR Gate is satisfied.
6. **Branch sync:** If behind `origin/$base`, merge `origin/$base` first (never rebase). Push after merge.

### PR Title Rules

- Format: `<type>(<scope>): <subject>` (Conventional Commits)
- type: `feat`/`fix`/`docs`/`chore`/`refactor`/`test`/`ci`/`perf`
- subject: imperative mood, under 70 chars, no capital, no period
- Branch prefix `feat/`/`fix/` must match title type

### PR Body Rules

- Template: the current runtime's `gwt-manage-pr/references/pr-body-template.md`
- Required sections: Summary, Changes, Testing, PR Readiness, Closing Issues, Related Issues, Checklist
- Conditional sections (remove if N/A): Context, Risk/Impact, Screenshots, Deployment
- Remove all `<!-- GUIDE: ... -->` comments from final output
- No `TODO` in required sections; derive content from diff/Issues/SPECs before asking user
- `Closing Issues`: `Closes #N` or `None` only. Bare `#N` without keyword is forbidden.
- Add reason to every unchecked checklist item

### Create/Update Commands

- Create: JSON operation `pr.create` with `params.base`, optional
  `params.head`, `params.title`, `params.body`, optional `params.labels`, and
  `params.draft`.
- Update: JSON operation `pr.edit` with `params.number`, optional
  `params.title`, optional `params.body`, and optional `params.add_labels`
  (at least one of them is required).
- Promote / demote: JSON operation `pr.ready` with `params.number` marks a
  Draft PR Ready for review — only after the Ready PR Gate passes.
  `pr.draft` converts a Ready PR back to Draft.
- Transport note: the current implementation may still call GitHub REST / `gh`
  internally, but agent-facing workflow should stay on gwtd JSON operations.

### Post-Create

After PR creation or push, automatically enter **fix** mode and use JSON
operations `pr.checks`, `pr.reviews`, `pr.review_threads`, `actions.logs`, and
`actions.job_logs` as the normal inspection path.

## Mode: Check

Detailed logic in `references/check-flow.md`.

**Read-only mode.** Do not create/switch branches, push, or create/edit PRs.

### Canonical Read Surface

- Current branch PR: `pr.current`
- PR detail: `pr.view`
- Checks: `pr.checks`
- Reviews: `pr.reviews`
- Unresolved threads: `pr.review_threads`
- Actions logs: `actions.logs` / `actions.job_logs`
- Actions re-run: `actions.rerun` (`run_id` + `failed_only`, or `job_id`)

### Output Contract

Human-readable summary using signal prefixes:

| Prefix | Action | Meaning |
|--------|--------|---------|
| `>>` | `CREATE PR` | Create a new PR |
| `>` | `PUSH ONLY` | Push to existing PR |
| `~` | `FIX` | Fix CI / reviews / conflicts on existing PR |
| `--` | `NO ACTION` | Nothing to do |
| `!!` | `MANUAL CHECK` | Manual check required |

Per-status templates:

Templates map directly from the Preflight 2×2 matrix:

- **N > 0, no open PR:** `>> CREATE PR -- <N> new commit(s) not covered by any PR.`
- **N > 0, open PR, clean merge state:** `> PUSH ONLY -- Unmerged PR #<number> open for <head>.` + PR URL
- **Open PR with blocking merge state:** `~ FIX -- PR #<number> is <mergeable>; resolve base sync/conflicts before push-only.`
- **N = 0, open PR:** `~ FIX -- PR #<number> open, checking CI/reviews/conflicts.`
- **N = 0, no open PR:** `-- NO ACTION -- No commits ahead of <base>, no open PR.`
- **Fallback:** `!! MANUAL CHECK -- Could not determine commit count.` + reason

Append `(!) Worktree has uncommitted changes.` when dirty.

### Commit Count (from Preflight Step 5)

The commit count `N = git rev-list --count origin/<base>..HEAD` is
computed in the Shared Preflight and is the primary routing signal.
Check mode simply reports it:

- `N > 0` + no open PR → `>> CREATE PR -- <N> new commit(s) not in any PR.`
- `N > 0` + open PR + clean merge state → `> PUSH ONLY -- Unmerged PR open.` + PR URL
- open PR + `mergeable: CONFLICTING|DIRTY|BEHIND` → `~ FIX -- PR is blocked by merge state.`
- `N = 0` + open PR → report CI / review / conflict status
- `N = 0` + no open PR → `-- NO ACTION`

The old "Post-Merge Commit Check" logic (merge_commit ancestry,
fallback chain) is subsumed by `git rev-list --count origin/<base>..HEAD`
which directly answers "does develop have all my work?" regardless of
PR state.

## Mode: Fix

Detailed logic in `references/fix-flow.md`.

### Inspection Targets

- **CI failures:** REST check-runs + GitHub Actions log extraction
- **Merge conflicts:** `mergeable` / `mergeStateStatus` fields (CONFLICTING/DIRTY/BEHIND)
- **Reviewer comments:** REST reviews + inline comments + issue comments (full text, no truncation)
- **Unresolved threads:** GraphQL only
- **Change requests:** Reviews with `CHANGES_REQUESTED` state

### Diagnosis Report (mandatory format)

```text
## Diagnosis Report: PR #<number>

**Merge Verdict: BLOCKED | CLEAR**
Blocking items: <N>

---

### BLOCKING

#### B1. [CATEGORY] <1-line title>
- **What:** Factual statement
- **Where:** file_path:line / check name / branch ref
- **Evidence:** Verbatim quote from output
- **Action:** Specific fix with file path/command
- **Auto-fix:** Yes | No (needs confirmation)

---

### INFORMATIONAL
#### I1. [CATEGORY] <1-line title>
- **What / Note**

---

**Summary:** <N> blocking, <M> informational.
```

**Categories:** `CONFLICT`, `BRANCH-BEHIND`, `CI-FAILURE`, `CHANGE-REQUEST`, `UNRESOLVED-THREAD`, `REVIEW-COMMENT`

**Classification:**

- BLOCKING: merge conflicts, BEHIND, CI failure/cancelled/timed_out, CHANGES_REQUESTED, unresolved threads, unanswered review comments (any reviewer comment with no reply)
- INFORMATIONAL: review comments that already have a reply, pending CI, outdated threads

**Each CHANGE-REQUEST, UNRESOLVED-THREAD, and UNANSWERED-COMMENT is a separate B-item.**

### Execution Path

1. All `Auto-fix: Yes` --> proceed directly to fix.
2. Any `Auto-fix: No` --> ask user about those items only.

### Fix Implementation

- Apply fixes, commit, push.
- For BRANCH-BEHIND: `git fetch origin <base> && git merge origin/<base> && git push`
- For CONFLICT: inspect both sides, resolve if clear, ask user if ambiguous.

### Comment Response Policy

> **No reviewer comment may be left unanswered or unresolved.**

- Every unresolved thread MUST receive a reply AND be resolved.
- Reply content (every comment gets a reply, even if no code change was needed):
  - Fixed: "Fixed: <what was done>." with commit reference.
  - Not applicable: "Not applicable: <reason why no change is needed>."
  - Already addressed: "Already addressed in commit <sha>: <summary>."
  - Acknowledged: "Acknowledged: <brief response>." for informational comments.
- After preparing the reply body, use JSON operation
  `pr.review_threads.reply_and_resolve` to reply to and resolve all unresolved
  threads on the PR.
- If that surface is unavailable, report the missing operation; do not bypass
  the mutation gate with direct GraphQL writes.
- **Verification:** After resolving, re-check that no unresolved
  threads remain. Unresolved threads block the Merge Verdict.

### Reviewer Notification (mandatory)

Post a PR summary comment via JSON operation `pr.comment`.

### Verify Fix (mandatory)

Re-run inspection with `--mode all`. Loop until exit code 0.
- CI pending --> poll 30s intervals until complete.
- After fix push, re-poll for new CI run.

### Loop Safety Guard

Same CI check fails 3 consecutive iterations --> report to user, ask continue/abort/change approach.

## Mode: Deliver (drive-to-merge)

Detailed logic in `references/deliver-flow.md`.

Deliver mode drives a verified, Ready PR all the way to a merged state: hand
the PR to the repository's merge automation, then run the existing Fix loop
against every blocker (CI / reviews / threads / conflicts) and poll until
`merged_at` is set. Deliver **composes** Fix mode — it does not reimplement
blocker resolution.

gwtd has no merge operation. The merge itself is performed by the
repository's merge automation (GitHub auto-merge, or a workflow that merges
non-Draft PRs once checks pass) or by a human. Deliver has two levers: JSON
operation `pr.ready` hands the PR to that automation, and JSON operation
`pr.draft` can explicitly hold a merge when the owner requests it,
because a Draft PR cannot be merged. It is not a prerequisite for routine pushes.

### Entry: autonomous execution or explicit manual request

For `launch_route: autonomous`, Deliver continues verified work through a
Ready PR and the existing CI auto-merge path until merged. Human visual
confirmation is not required, and a Draft PR is not the terminal outcome for
passing work. Manual Deliver is opt-in only and never auto-routed: the user must
explicitly ask to deliver / drive to merge / merge / land / ship the PR. If no
open PR exists, first use Create with the Ready PR Gate, then drive that PR.

### Hard PR Gate (mandatory before handing the PR to merge automation)

Do **not** hand the PR to merge automation until the Ready PR Gate passes for
the PR scope. Deliver applies a stricter gate than Create/Fix because merge
automation removes the last human checkpoint:

- `gwt-verify --mode pre-pr` returns `Overall: PASS`
- `User Verification Result` is `confirmed`, `n/a`, or `n/a (autonomous)`.
  Existing autonomous PRs may retain the legacy `deferred (autonomous execution)`
  body value with fresh passing evidence. `pending`, `rejected(<reason>)`, and
  `skipped(<reason>)` do not authorize delivery. Autonomous UI work requires
  `Agent Visual Check: pass` plus the same record's measured passing headed
  Chromium results for dark and light themes.
- the PR is a releaseable slice with no known blockers

If verification is `pending`, do **not** hand new work over through `pr.ready`.
Route the failure for repair (back to the TDD loop, `gwt-verify`, or
`gwt-discussion`). Never downgrade `pending` to `skipped` to pass the gate.

### Preserve the repository's delivery policy

Keep auto-merge enabled across routine pushes and base synchronization. Run
fresh `gwt-verify --mode pre-pr` before every code-changing push. Use JSON
operation `pr.update_branch` for a BEHIND PR; when the PM owns synchronization,
hand it off on the Board instead of updating the branch concurrently.

Do not require an unavailable merge operation or a Draft/Ready cycle for each
push. An explicit owner-requested hold uses `pr.draft`; resume through
`pr.ready` only after the Ready PR Gate passes again.

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

The same blocker (same CI check name, same unresolved thread, same conflict)
surviving 3 consecutive drive iterations → stop, report what was tried each
iteration, and ask the user **continue** / **abort** / **change approach**. Do
not loop indefinitely; the merged-state poll is bounded and hands off via
`board.post` (`params.kind:"blocked"`) when CI stays pending.

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

- `references/create-flow.md` -- PR creation workflow details
- `references/check-flow.md` -- PR status checking details
- `references/fix-flow.md` -- CI/conflict/review fix details
- `references/deliver-flow.md` -- drive-to-merge (auto-merge + Fix loop + merged watch) details
- `references/pr-body-template.md` -- PR body template
