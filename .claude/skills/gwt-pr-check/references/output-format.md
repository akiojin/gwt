# Output Contract

Return a human-readable summary by default.

Do not return raw JSON as the default output.
If JSON is explicitly requested by the user, append it after the human summary.

## Status values

- `NO_PR`
- `UNMERGED_PR_EXISTS`
- `CLOSED_UNMERGED_ONLY`
- `ALL_MERGED_WITH_NEW_COMMITS`
- `ALL_MERGED_NO_PR_DIFF`
- `CHECK_FAILED`

## Action values

- `CREATE_PR`
- `PUSH_ONLY`
- `NO_ACTION`
- `MANUAL_CHECK`

## Language rule

- Follow the user's input language for all headings and messages.
- If the language is ambiguous, use English.

## Default output template

Output 1-3 lines using a signal prefix + action keyword on line 1.

| Prefix | Action | Meaning |
| --- | --- | --- |
| `>>` | `CREATE PR` | Create a new PR |
| `>` | `PUSH ONLY` | Push to existing PR |
| `--` | `NO ACTION` | Nothing to do |
| `!!` | `MANUAL CHECK` | Manual check required |

## Per-status format

- **NO_PR**:
  `>> CREATE PR — No PR exists for <head> -> <base>.`
- **UNMERGED_PR_EXISTS** (2 lines):

  ```text
  > PUSH ONLY — Unmerged PR open for `<head>`.
     PR: #<number> <url>
  ```

- **CLOSED_UNMERGED_ONLY** (2 lines):

  ```text
  >> CREATE PR — No open PR exists for <head> -> <base>; only closed unmerged PRs were found.
     Last closed PR: #<number> <url>
  ```

- **ALL_MERGED_WITH_NEW_COMMITS** (2 lines):

  ```text
  >> CREATE PR — <N> new commit(s) after last merge (#<pr_number>).
     head: <head> -> base: <base>
  ```

- **ALL_MERGED_NO_PR_DIFF**:
  `-- NO ACTION — All PRs merged, no PR-worthy diff on <head>.`
- **CHECK_FAILED** (2 lines):

  ```text
  !! MANUAL CHECK — Could not determine PR status.
     Reason: <reason>
     head: <head> -> base: <base>
  ```

Append the following line **only** when the worktree is dirty:

```text
   (!) Worktree has uncommitted changes.
```

## Status-to-action mapping (must use)

| Status | Prefix | Action | Template |
| --- | --- | --- | --- |
| `NO_PR` | `>>` | `CREATE PR` | No PR exists |
| `UNMERGED_PR_EXISTS` | `>` | `PUSH ONLY` | Unmerged PR open |
| `CLOSED_UNMERGED_ONLY` | `>>` | `CREATE PR` | Only closed unmerged PRs exist |
| `ALL_MERGED_WITH_NEW_COMMITS` | `>>` | `CREATE PR` | N new commit(s) |
| `ALL_MERGED_NO_PR_DIFF` | `--` | `NO ACTION` | All PRs merged |
| `CHECK_FAILED` | `!!` | `MANUAL CHECK` | Could not determine |

## Example outputs

**NO_PR:**

```text
>> CREATE PR — No PR exists for `feature/my-branch` -> `develop`.
```

**UNMERGED_PR_EXISTS:**

```text
> PUSH ONLY — Unmerged PR open for `feature/my-branch`.
   PR: #456 https://github.com/org/repo/pull/456
```

**CLOSED_UNMERGED_ONLY:**

```text
>> CREATE PR — No open PR exists for `feature/my-branch` -> `develop`; only closed unmerged PRs were found.
   Last closed PR: #455 https://github.com/org/repo/pull/455
```

**ALL_MERGED_WITH_NEW_COMMITS:**

```text
>> CREATE PR — 3 new commit(s) after last merge (#123).
   head: feature/my-branch -> base: develop
```

**ALL_MERGED_NO_PR_DIFF:**

```text
-- NO ACTION — All PRs merged, no PR-worthy diff on `feature/my-branch`.
```

**CHECK_FAILED:**

```text
!! MANUAL CHECK — Could not determine PR status.
   Reason: Could not resolve merge commit and fallback comparison failed
   head: feature/my-branch -> base: develop
```

**With dirty worktree (appended to any status):**

```text
>> CREATE PR — 3 new commit(s) after last merge (#123).
   head: feature/my-branch -> base: develop
   (!) Worktree has uncommitted changes.
```
