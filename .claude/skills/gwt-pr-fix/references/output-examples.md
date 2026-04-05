# PR Fix Output Examples

## Diagnosis Report

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
- **Action:** Edit `src/utils/parser.ts:42` — change `parseInt(value)` to pass the correct type
- **Auto-fix:** Yes

#### B2. [CHANGE-REQUEST] @reviewer1 requests error handling
- **What:** Reviewer requested try-catch around API call
- **Where:** `src/api/client.ts:88`
- **Evidence:** "@reviewer1: Please wrap this fetch call in a try-catch block to handle network errors gracefully."
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
- **What / Note:** @reviewer2 suggested extracting a helper function — non-blocking style preference

---

**Summary:** 3 blocking items to fix, 1 informational item noted.
```

## Text Output

```text
PR #123: Comprehensive Check Results
============================================================

MERGE STATUS
------------------------------------------------------------
Mergeable: CONFLICTING
Merge State: DIRTY
Base: main <- Head: feature/my-branch
Action Required: Resolve conflicts before merging

CHANGE REQUESTS
------------------------------------------------------------
From @reviewer1 (2025-01-15):
  "Please fix these issues..."

UNRESOLVED REVIEW THREADS
------------------------------------------------------------
[1] src/main.ts:42
    Thread ID: PRRT_xxx123
    @reviewer1: This needs refactoring because the current
    implementation violates the single responsibility principle.

[2] src/utils.ts:15
    Thread ID: PRRT_xxx456
    @reviewer2: Consider using a more descriptive variable name here.

CI FAILURES
------------------------------------------------------------
Check: build
Details: https://github.com/...
Failure snippet:
  Error: TypeScript compilation failed
  ...
============================================================
```

## Reply and Resolve Output

```text
OK: PRRT_xxx123 (src/main.ts:42)
OK: PRRT_xxx456 (src/utils.ts:15)

Result: 2 resolved, 0 failed, 2 total
```

## JSON Output

```json
{
  "pr": "123",
  "conflicts": {
    "hasConflicts": true,
    "mergeable": "CONFLICTING",
    "mergeStateStatus": "DIRTY",
    "baseRefName": "main",
    "headRefName": "feature/my-branch"
  },
  "changeRequests": [
    {
      "id": 123456,
      "reviewer": "reviewer1",
      "body": "Please fix these issues...",
      "submittedAt": "2025-01-15T12:00:00Z"
    }
  ],
  "unresolvedThreads": [
    {
      "id": "PRRT_xxx123",
      "path": "src/main.ts",
      "line": 42,
      "comments": [
        {"author": "reviewer1", "body": "This needs refactoring because..."}
      ]
    }
  ],
  "ciFailures": [
    {
      "name": "build",
      "status": "ok",
      "logSnippet": "..."
    }
  ]
}
```
