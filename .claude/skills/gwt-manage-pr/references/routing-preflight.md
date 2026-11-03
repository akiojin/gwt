# gwt-manage-pr — Unified PR Lifecycle Manager

## Overview

Single skill for the full PR lifecycle: create, check status, fix blockers, and deliver (drive to merge). Auto-detects the appropriate mode from current branch PR state, or accepts an explicit mode from the user. Deliver is automatic for autonomous execution after the Ready PR Gate passes; manual launches require an explicit delivery request.

Use the current user's language for decision summaries, blocker reports, and
next-step guidance returned from this workflow.

Read-only `gh` commands are allowed and recorded on the shared GitHub budget
ledger. Prefer gwtd JSON operations such as `pr.list` and `issue.view` when
cached data or workflow lifecycle context is needed. GitHub mutations must
use JSON-envelope operations, including `pr.ready` for handing a
PR to merge automation, and `actions.rerun` for rerunning
CI. gwtd has no merge operation. All verification and Ready PR gates still
apply. The current
implementation may still use GitHub REST / `gh` internally as transport, while
GraphQL remains the transport for unresolved review threads and thread
reply/resolve.

## Final Report Contract

Final user-facing reports should state whether a PR was
created, updated, fixed, delivered (driven to merge), or left unchanged. When a
PR was created, pushed to, fixed, or delivered, include a `PR Update Summary`
derived from the actual diff, commit range, and PR body. Do not invent changes
from intent alone.

Include useful facts from this suggested `PR Update Summary`:

- PR number and URL
- base/head branches
- commit count or updated commit range
- 2-5 bullets describing what the PR updates
- related Issue/SPEC numbers, or `None`
- verification and CI/review status

For **check** and **NO ACTION** results, explicitly say no PR update was made.
If an existing PR is only inspected, summarize the current PR status separately
from `PR Update Summary` so the report does not imply new changes were pushed.

Example final report shape:

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
2. **Branch protection:** `main` accepts `develop` or same-repository `release/vX.Y.Z` snapshots whose source tree matches their merge base with `develop`. The required `check-source-branch` workflow proves the snapshot; a release branch name alone is insufficient. Refuse other source branches. Never create or advance release branches locally; Prepare Release owns them.
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
