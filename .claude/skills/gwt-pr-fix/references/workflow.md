# PR Fix Workflow (Detailed Steps)

## Step 1: Verify gh authentication

- Run `gh auth status` in the repo with escalated scopes (workflow/repo).
- If unauthenticated, ask the user to log in before proceeding.

## Step 2: Resolve the PR

- Prefer the current branch PR through a REST head-branch lookup.
- If the user provides a PR number or URL, use that directly.

## Step 3: Inspect based on mode

### Conflicts Mode (`--mode conflicts`)

- Check `mergeable` and `mergeStateStatus` fields.
- If `CONFLICTING` or `DIRTY`, report conflict details.
- If `BEHIND`, report that the base branch advanced and a base-branch merge is required.
- Default resolution path is `git fetch origin <base> && git merge origin/<base>`.
- Do not recommend rebase for gwt PR maintenance.

### Reviews Mode (`--mode reviews`)

- Fetch reviews with `CHANGES_REQUESTED` state.
- Fetch ALL reviewer comments (review summaries, inline review comments, issue comments) without truncation through REST.
- Fetch unresolved review threads using GraphQL only.
- Display reviewer, comment body (full text), file path, and line number.
- Decide if reviewer feedback requires action (any change request, unresolved thread, or reviewer comment).

### Checks Mode (`--mode checks`)

- Run bundled script to inspect failing CI checks through REST check-runs.
- Add `--required-only` to limit output when a reliable required-only source is available; otherwise report that all checks are being shown.
- Fetch GitHub Actions logs and extract failure snippets.
- For external checks (Buildkite, etc.), report URL only.

### All Mode (`--mode all`)

- Run all inspections above.

## Step 4: Produce Diagnosis Report (mandatory format)

Output MUST use this exact structure:

```text
## Diagnosis Report: PR #<number>

**Merge Verdict: BLOCKED | CLEAR**
Blocking items: <N>

---

### BLOCKING

#### B1. [CATEGORY] <1-line title>
- **What:** Factual statement (no speculation)
- **Where:** file_path:line_number / check name / branch ref
- **Evidence:** Verbatim quote from script output
- **Action:** Specific fix (file path, command, or code change — at least one required)
- **Auto-fix:** Yes | No (needs confirmation)

#### B2. [CATEGORY] <1-line title>
...

---

### INFORMATIONAL
#### I1. [CATEGORY] <1-line title>
- **What / Note**

---

**Summary:** <N> blocking items to fix, <M> informational items noted.
```

### Classification rules

- **BLOCKING:** CONFLICTING/DIRTY merge state, BEHIND, CI failure/cancelled/timed_out/action_required, CHANGES_REQUESTED, unresolved review threads
- **INFORMATIONAL:** Comments without change requests, pending CI, outdated review threads

### Category labels

`CONFLICT`, `BRANCH-BEHIND`, `CI-FAILURE`, `CHANGE-REQUEST`, `UNRESOLVED-THREAD`, `REVIEW-COMMENT`

### Auto-fix judgment

- **Auto-fix: Yes** — CI-FAILURE code fixes, reviewer instructions that the LLM can address with high confidence
- **Auto-fix: No (needs confirmation)** — merge conflicts you cannot resolve with high confidence, low-confidence reviewer instructions, changes requiring design decisions

**Each CHANGE-REQUEST and each UNRESOLVED-THREAD is a separate B-item.** Do not combine multiple threads or requests into one item.

## Step 5: Decide execution path

- If ALL blocking items have `Auto-fix: Yes` -> display Diagnosis Report and proceed directly to step 6.
- If ANY blocking item has `Auto-fix: No` -> create a concise plan referencing B-item IDs and ask the user only about those ambiguous items before proceeding.

## Step 6: Implement fixes

- Apply the fixes, summarize diffs/tests.
- After applying fixes, commit changes and push to the PR branch.
- Verify push succeeded before proceeding to step 7.
- **After implementing fixes, proceed to step 7 to reply and resolve ALL threads.**
- For BRANCH-BEHIND items, see [Fix Strategies](#fix-strategies).
- For CONFLICT items, see [Fix Strategies](#fix-strategies).

## Step 7: Reply to ALL reviewer comments and resolve threads (mandatory)

- **CRITICAL:** Every unresolved review thread MUST receive a reply before resolution. No thread may be silently resolved or left unaddressed.
- For each unresolved thread, prepare a reply:
  - If addressed: describe what was done (e.g., "Fixed: refactored the method as suggested in commit abc1234.")
  - If intentionally not addressed: explain the reason (e.g., "Not addressed: this is by design because ...")
- Use `--reply-and-resolve` with a JSON array covering ALL unresolved threads.
- The script validates completeness and rejects the operation if any thread is missing a reply.
- Requires `Repository Permissions > Contents: Read and Write`.
- Resolve threads at this point (after code fix is pushed). Do not wait for CI completion to resolve threads.
- GraphQL remains only for unresolved review threads and thread reply/resolve in this workflow; if GitHub rate-limits those mutations, back off and retry instead of fabricating a REST replacement.

## Step 8: Notify reviewers (mandatory)

- With `--add-comment "message"`, post a comment to the PR through REST first.
- Include a summary of what was fixed (list each B-item and the action taken).
- This step is not optional — always notify reviewers after fixes are applied.
- If the REST comment path fails unexpectedly, fall back to `gh pr comment`.

## Step 9: Verify fix (mandatory — do not skip)

- Re-run the inspection script with `--mode all` (regardless of initial mode).
- Exit code 0 -> all resolved -> report success to user.
- Exit code 1 -> issues remain -> go back to step 4 with new output.
- No iteration limit. Continue until all issues are resolved.
- CI still pending/queued -> poll at 30-second intervals (no timeout) until ALL checks complete.
- Wait for ALL CI checks to complete before starting fixes (pushing resets pending checks).
- After fix push, re-enter polling to wait for new CI run to complete.

## Loop Safety Guard

- If the **same CI check name** (e.g., `build`) fails **3 consecutive iterations**:
  1. Report to user: which check, what was tried in each iteration, what keeps failing.
  2. Ask user to choose: **continue** / **abort** / **change approach**.
  3. Only proceed after explicit user decision.
- This prevents oscillation loops where fix A breaks B and fix B breaks A.
- Different checks failing in different iterations do NOT trigger the guard (e.g., `build` fails -> fixed -> `lint` fails is normal progression, not oscillation).

## Fix Strategies

### BRANCH-BEHIND

- Default strategy: `git fetch origin <base> && git merge origin/<base> && git push`
- This is automatic when the merge is clean.
- If merge results in conflicts, switch to CONFLICT handling below.

### CONFLICT

- First inspect the conflicting files and reason about the behavioral impact of each side; do not resolve by mechanically taking one side.
- If the correct merge is clear and low-risk, resolve it, run the relevant checks, and push.
- If the correct merge is not clear, present the conflict summary and ask the user before proceeding.
