# PR Fix Flow (Detailed)

## Step 1: Verify gh authentication

- Run `gh auth status` in the repo with escalated scopes (workflow/repo).
- If `GH_TOKEN` / `GITHUB_TOKEN` is set, allow direct REST auth without blocking on `gh auth status`.
- If unauthenticated, ask the user to log in before proceeding.

## Step 2: Resolve the PR

- Prefer the current branch PR through a REST head-branch lookup.
- If the user provides a PR number or URL, use that directly.

## Step 3: Inspect based on mode

### Conflicts Mode (`--mode conflicts`)

- Check `mergeable` and `mergeStateStatus` fields.
- If `CONFLICTING` or `DIRTY`, report conflict details.
- If `BEHIND`, report that the base branch advanced and a base-branch merge is required.
- Default resolution: `git fetch origin <base> && git merge origin/<base>`.
- Do not recommend rebase for gwt PR maintenance.

### Reviews Mode (`--mode reviews`)

- Fetch reviews with `CHANGES_REQUESTED` state.
- Fetch ALL reviewer comments (review summaries, inline review comments, issue comments) without truncation through REST.
- Fetch unresolved review threads using GraphQL only.
- Display reviewer, comment body (full text), file path, and line number.

### Checks Mode (`--mode checks`)

- Inspect failing CI checks through REST check-runs.
- Add `--required-only` to limit output when available.
- Fetch GitHub Actions logs and extract failure snippets.
- For external checks (Buildkite, etc.), report URL only.

### All Mode (`--mode all`)

- Run all inspections above.

## Step 4: Produce Diagnosis Report

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
- **Action:** Specific fix (file path, command, or code change)
- **Auto-fix:** Yes | No (needs confirmation)

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

- **Yes:** CI-FAILURE code fixes, reviewer instructions addressable with high confidence
- **No (needs confirmation):** Merge conflicts you cannot resolve confidently, low-confidence reviewer instructions, design decisions

**Each CHANGE-REQUEST and each UNRESOLVED-THREAD is a separate B-item.**

## Step 5: Decide execution path

- All blocking items `Auto-fix: Yes` --> proceed directly to step 6.
- Any blocking item `Auto-fix: No` --> ask user about those items only.

## Step 6: Implement fixes

- Apply the fixes, summarize diffs/tests.
- After applying fixes, commit changes and push to the PR branch.
- Verify push succeeded before proceeding to step 7.

### Fix Strategies

#### BRANCH-BEHIND

- Default: `git fetch origin <base> && git merge origin/<base> && git push`
- Automatic when merge is clean.
- If conflicts arise, switch to CONFLICT handling.

#### CONFLICT

- Inspect conflicting files and reason about behavioral impact of each side.
- If correct merge is clear and low-risk, resolve it, run checks, push.
- If not clear, present conflict summary and ask user.

## Step 7: Reply to ALL reviewer comments and resolve threads (mandatory)

> **CRITICAL:** Every unresolved review thread MUST receive a reply before resolution.

- For each unresolved thread:
  - Addressed: "Fixed: <what was done in commit abc1234>."
  - Not addressed: "Not addressed: <reason>."
- Use `--reply-and-resolve` with JSON array covering ALL threads.
- Script validates completeness; rejects if any thread missing.
- Requires `Repository Permissions > Contents: Read and Write`.
- Resolve threads after code fix is pushed. Do not wait for CI.
- GraphQL only for thread operations. If rate-limited, back off and retry.

### `--reply-and-resolve` JSON format

```json
[
  {"threadId": "PRRT_xxx123", "body": "Fixed: refactored the method as suggested."},
  {"threadId": "PRRT_xxx456", "body": "Not addressed: this is intentional because ..."}
]
```

## Step 8: Notify reviewers (mandatory)

- Post comment via REST: `POST /repos/<owner>/<repo>/issues/<pr_number>/comments`
- Include summary of what was fixed (list each B-item and action taken).
- Fallback: `gh pr comment`.
- This step is not optional.

## Step 9: Verify fix (mandatory)

- Re-run inspection with `--mode all` (regardless of initial mode).
- Exit code 0 --> all resolved --> report success.
- Exit code 1 --> issues remain --> go back to step 4.
- No iteration limit. Continue until all resolved.
- CI pending/queued --> poll at 30-second intervals until ALL checks complete.
- After fix push, re-enter polling for new CI run.

## Loop Safety Guard

- Same CI check name fails 3 consecutive iterations:
  1. Report which check, what was tried each iteration, what keeps failing.
  2. Ask user: **continue** / **abort** / **change approach**.
  3. Only proceed after explicit user decision.
- Different checks failing in different iterations do NOT trigger the guard.

## Anti-Patterns (Prohibited)

| Prohibited | Required Alternative |
|---|---|
| "We should look into..." | "Edit `path/file.ts:42` to..." |
| "There seem to be some issues" | "3 blocking items detected" |
| "This might be causing..." | "Root cause: `<error from log>`" |
| "Consider fixing..." / "It looks like..." | "Action: Fix `<what>` in `<where>`" |
| "Various CI checks are failing" | "2 CI checks failing: `build`, `lint`" |
| "Some reviewers have concerns" | "@reviewer1 requested: `<quote>`" |
| "I'll try to fix this" | "Action: \<specific fix\>" |

### Structural Prohibitions

- Prose paragraphs for reporting -- use B1/I1 item format exclusively.
- Omitting the Evidence field in any BLOCKING item.
- Combining multiple independent problems into a single item.
- Omitting file paths or line numbers when the script output contains them.

## Bundled Script Reference

```bash
# Inspect all (CI, conflicts, reviews)
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>"

# CI checks only
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --mode checks

# Reviews only
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --mode reviews

# Reply and resolve all threads
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --reply-and-resolve '[
  {"threadId":"PRRT_xxx123","body":"Fixed: refactored as suggested."}
]'

# Notify reviewers
python3 ".claude/skills/gwt-pr-fix/scripts/inspect_pr_checks.py" --repo "." --pr "<number>" --add-comment "Fixed all issues. Please re-review."
```

## Output Examples

### Diagnosis Report

```text
## Diagnosis Report: PR #123

**Merge Verdict: BLOCKED**
Blocking items: 3

---

### BLOCKING

#### B1. [CI-FAILURE] TypeScript build fails
- **What:** `build` check failed with compilation error
- **Where:** `src/utils/parser.ts:42` / check: `build`
- **Evidence:** `error TS2345: Argument of type 'string' is not assignable to parameter of type 'number'.`
- **Action:** Edit `src/utils/parser.ts:42` -- change `parseInt(value)` to pass the correct type
- **Auto-fix:** Yes

#### B2. [CHANGE-REQUEST] @reviewer1 requests error handling
- **What:** Reviewer requested try-catch around API call
- **Where:** `src/api/client.ts:88`
- **Evidence:** "@reviewer1: Please wrap this fetch call in a try-catch block."
- **Action:** Add try-catch in `src/api/client.ts:88` around the `fetch()` call
- **Auto-fix:** Yes

#### B3. [CONFLICT] Merge conflict with main
- **What:** 2 files have merge conflicts
- **Where:** `src/config.ts`, `src/index.ts` / branch: `main`
- **Evidence:** `Mergeable: CONFLICTING, Merge State: DIRTY`
- **Action:** Merge `origin/main` and resolve conflicts in listed files
- **Auto-fix:** No (needs confirmation)

---

### INFORMATIONAL
#### I1. [REVIEW-COMMENT] Code style suggestion
- **What / Note:** @reviewer2 suggested extracting a helper function -- non-blocking style preference

---

**Summary:** 3 blocking items to fix, 1 informational item noted.
```

### Reply and Resolve Output

```text
OK: PRRT_xxx123 (src/main.ts:42)
OK: PRRT_xxx456 (src/utils.ts:15)

Result: 2 resolved, 0 failed, 2 total
```
