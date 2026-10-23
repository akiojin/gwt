## Mode: Fix

Detailed logic in [references/fix-flow.md](fix-flow.md).

### Inspection Targets

- **CI failures:** REST check-runs + GitHub Actions log extraction
- **Merge conflicts:** `mergeable` / `mergeStateStatus` fields (CONFLICTING/DIRTY/BEHIND)
- **Reviewer comments:** REST reviews + inline comments + issue comments (full text, no truncation)
- **Unresolved threads:** GraphQL only
- **Change requests:** Reviews with `CHANGES_REQUESTED` state

### Diagnosis Report (example format)

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

- Every unresolved thread MUST be addressed and resolved. Existing identical
  replies are not repeated, and outdated threads are resolved without a new reply.
- Reply content for current threads, even if no code change was needed:
  - Fixed: "Fixed: <what was done>." with commit reference.
  - Not applicable: "Not applicable: <reason why no change is needed>."
  - Already addressed: "Already addressed in commit <sha>: <summary>."
  - Acknowledged: "Acknowledged: <brief response>." for informational comments.
- After preparing each plain text reply body, use JSON operation
  `pr.review_threads.reply_and_resolve` once per thread with `number`,
  `thread_id`, and `body`. JSON array bodies are rejected.
- Always specify `thread_id` for an individual response. Omitting `thread_id`
  processes all unresolved threads on the PR with the same `body`, preserving
  the identical-reply and outdated-thread behavior above.
- If that surface is unavailable, report the missing operation; do not bypass
  the mutation gate with direct GraphQL writes.
- **Verification:** After resolving, re-check that no unresolved
  threads remain. Unresolved threads block the Merge Verdict.

### Reviewer Notification (mandatory)

Post a PR summary comment via JSON operation `pr.comment`.

### Verify Fix (mandatory)

Re-run inspection with `--mode all`. Loop until exit code 0.
- CI pending --> use the bounded checks and wait handling in [references/fix-flow.md](fix-flow.md).
- After fix push, re-poll for new CI run.

### Loop Safety Guard

Same CI check fails 3 consecutive iterations --> report to user, ask continue/abort/change approach.
