> **ℹ️ TUI MIGRATION NOTE**: This SPEC describes backend/gwt-core functionality unaffected by the gwt-tui migration (SPEC-1776). No changes required.

# Feature Specification: gwt-pr-check 統合ステータスレポート

## Background

gwt-pr-check は PR の「次のアクション推奨」のみを出力するスキルとして設計された。しかしユーザーが `/gwt-pr-check` を実行する動機は「PR が今どういう状態か」を把握することであり、推奨アクションだけでは情報が不足している。

現状の問題:
- `PUSH ONLY` と表示されても、CI が通っているのか、レビューが承認されているのか、コンフリクトがあるのかが分からない
- PR の総合的な状態を知るには `gwt-pr-fix` を別途実行するか、GitHub Web UI を開く必要がある
- gwt-pr-fix の `inspect_pr_checks.py` は既に CI・マージ・レビュー情報を取得する機能を持つが、gwt-pr-check は活用していない

## User Stories

### US-1: PR の総合ステータスを一目で把握する (P0)

**As a** developer working on a feature branch,
**I want** gwt-pr-check to show me the full PR status at a glance,
**so that** I can immediately see what needs attention without opening the GitHub Web UI.

**Acceptance Scenarios:**

1. Given an open PR with passing CI, clean merge state, and pending review, when gwt-pr-check runs, then the output shows CI passed, no conflicts, and review pending in a structured format.
2. Given an open PR with 2 failing CI checks, when gwt-pr-check runs, then the output lists the failing check names and their status.
3. Given an open PR that is behind the base branch, when gwt-pr-check runs, then the output shows how many commits behind and whether conflicts exist.
4. Given an open PR with an approved review, when gwt-pr-check runs, then the output shows the approval count and reviewer names.
5. Given an open PR with change requests, when gwt-pr-check runs, then the output shows the change request count and reviewer names.

### US-2: 推奨アクションを引き続き提供する (P0)

**As a** developer,
**I want** gwt-pr-check to still tell me the recommended next action,
**so that** I know whether to push, create a PR, or take no action.

**Acceptance Scenarios:**

1. Given no PR exists, when gwt-pr-check runs, then it shows `CREATE PR` alongside the status report.
2. Given an unmerged PR exists, when gwt-pr-check runs, then it shows the PR URL alongside the full status report (not just `PUSH ONLY`).
3. Given all PRs are merged with new commits, when gwt-pr-check runs, then it shows `CREATE PR` with the commit count.

### US-3: GraphQL rate limit 時にも動作する (P1)

**As a** developer in a high-activity repository,
**I want** gwt-pr-check to fall back to REST API when GraphQL is rate-limited,
**so that** I always get a useful status report.

**Acceptance Scenarios:**

1. Given GraphQL rate limit is exceeded, when gwt-pr-check runs, then it falls back to REST API for CI checks and merge state.
2. Given REST API returns review data, when GraphQL is unavailable, then review counts are still displayed (thread-level detail may be omitted).
3. Given both GraphQL and REST fail for a specific section, when gwt-pr-check runs, then that section shows "Unavailable" instead of failing entirely.

### US-4: PR が存在しない場合も有用な情報を提供する (P1)

**As a** developer who hasn't created a PR yet,
**I want** gwt-pr-check to show worktree state and unpushed commits,
**so that** I know the full picture before creating a PR.

**Acceptance Scenarios:**

1. Given no PR exists and there are uncommitted changes, when gwt-pr-check runs, then it shows the worktree dirty warning and commit count ahead of base.
2. Given no PR exists and the branch is clean, when gwt-pr-check runs, then it shows `CREATE PR` with the commit count ahead of base.

## Edge Cases

- GraphQL rate limit exceeded but REST still works
- REST rate limit exceeded but GraphQL still works
- Both APIs rate-limited for different endpoints
- PR exists but CI checks haven't started yet (queued)
- PR has no reviewers assigned
- PR has draft status
- Multiple open PRs for the same branch (edge case in gwt workflow)
- PR base branch has been deleted

## Functional Requirements

- **FR-001**: gwt-pr-check MUST display CI check status (passed/failed/pending counts and failing check names) when a PR exists.
- **FR-002**: gwt-pr-check MUST display merge state (mergeable, conflicts, behind count) when a PR exists.
- **FR-003**: gwt-pr-check MUST display review state (approved/changes-requested/pending counts) when a PR exists.
- **FR-004**: gwt-pr-check MUST continue to display the recommended action (CREATE_PR / PUSH_ONLY / NO_ACTION / MANUAL_CHECK).
- **FR-005**: gwt-pr-check MUST fall back to REST API when GraphQL is rate-limited.
- **FR-006**: gwt-pr-check MUST degrade gracefully when a specific API section fails, showing "Unavailable" for that section.
- **FR-007**: The existing `check_pr_status.py` script MUST be extended (not replaced) to add the new status sections.
- **FR-008**: The SKILL.md output template MUST be updated to define the new integrated output format.
- **FR-009**: gwt-pr-check MUST reuse gwt-pr-fix's `inspect_pr_checks.py` capabilities where practical, or extract shared utilities.

## Non-Functional Requirements

- **NFR-001**: The total execution time MUST NOT exceed 15 seconds under normal API conditions.
- **NFR-002**: The output MUST remain scannable — no more than 10 lines for a typical clean PR.

## Output Format (Proposed)

```text
PR #1772: feature/issue-1771 -> develop (OPEN)
   CI:     6 passed, 0 failed, 4 pending
   Merge:  Clean (0 behind base)
   Review: 0 approved, 0 changes requested, 0 pending
   > PUSH ONLY — Unmerged PR #1772
   (!) Worktree has uncommitted changes.
```

When no PR exists:

```text
No PR for feature/issue-1771 -> develop
   Commits: 3 ahead of base
   >> CREATE PR
   (!) Worktree has uncommitted changes.
```

When CI has failures:

```text
PR #1772: feature/issue-1771 -> develop (OPEN)
   CI:     4 passed, 2 FAILED [build, lint], 0 pending
   Merge:  Clean (0 behind base)
   Review: 1 approved (@reviewer1), 0 changes requested
   > PUSH ONLY — Unmerged PR #1772
```

## Success Criteria

1. `/gwt-pr-check` shows CI status, merge state, and review state in one output
2. All existing test scenarios for check_pr_status.py continue to pass
3. GraphQL rate limit does not cause complete failure
4. Output is 10 lines or fewer for typical cases
