# SPEC-4: GitHub Integration — Issues, PRs, Git View, Branch Linkage

## Background

gwt integrates with GitHub via the `gh` CLI for Issue management, PR tracking, and branch linkage. The Issue tab and branch-issue linkage are already implemented. Git View (file diffs, commit history) existed in the old TUI (v6.30.3) but was removed during the rewrite and needs to be restored. A PR dashboard tab needs to be added as a new management panel tab.

## User Stories

### US-1 (P0): Browse and Search Issues — IMPLEMENTED

As a developer, I want to browse and search GitHub Issues from within gwt so that I can manage work items without leaving the terminal.

**Acceptance Scenarios:**

- AC-1.1: Issue list displays title, number, state, labels, and assignee
- AC-1.2: Search filters issues by keyword in title/body
- AC-1.3: Issue list refreshes on tab focus

### US-2 (P0): View Issue Detail with Markdown — IMPLEMENTED

As a developer, I want to view full Issue details rendered as markdown so that I can read descriptions, comments, and linked references.

**Acceptance Scenarios:**

- AC-2.1: Issue detail renders markdown (headings, lists, code blocks, links)
- AC-2.2: Labels and assignees display in the detail header
- AC-2.3: Pressing Esc returns to the Issue list

### US-3 (P0): See Linked Issues per Branch — IMPLEMENTED

As a developer, I want to see which Issues are linked to the current branch so that I can track what work belongs to my branch.

**Acceptance Scenarios:**

- AC-3.1: Branch detail shows linked Issues extracted from branch name or commit messages
- AC-3.2: Clicking a linked Issue navigates to Issue detail

### US-4 (P1): View Git Status (Staged/Unstaged Files, Diffs, Commits) -- IMPLEMENTED

As a developer, I want a Git View tab showing file status, diffs, and recent commits so that I can review changes without leaving gwt.

**Acceptance Scenarios:**

- AC-4.1: Git View tab appears in the management panel tab bar
- AC-4.2: File list shows staged files with `[S]`, unstaged with `[U]`, untracked with `[?]`
- AC-4.3: Selecting a file shows its diff (lazy-loaded, max 50 lines initially)
- AC-4.4: Last 5 commits display with hash, author, date, and message
- AC-4.5: Divergence status shows ahead/behind counts relative to remote
- AC-4.6: If a PR exists for the branch, a PR link is shown

### US-5 (P1): View PR Status, CI Checks, Merge State -- IMPLEMENTED

As a developer, I want a PR dashboard tab showing my PRs with CI status and merge readiness so that I can monitor PR progress.

**Acceptance Scenarios:**

- AC-5.1: PR dashboard tab appears in the management panel tab bar
- AC-5.2: PR list shows title, number, state, CI status icon, and merge state
- AC-5.3: Selecting a PR shows detail with CI check badges
- AC-5.4: PR detail shows review status (approved, changes requested, pending)
- AC-5.5: PR data refreshes on tab focus

### US-6 (P1): Launch Agent from Issue Detail — IMPLEMENTED

As a developer, I want to launch an agent session from an Issue detail view so that I can start working on an issue immediately.

**Acceptance Scenarios:**

- AC-6.1: Shift+Enter on Issue detail opens agent launch wizard
- AC-6.2: Agent launch pre-fills Issue context

## Functional Requirements

| ID | Requirement | Priority | Status |
|----|-------------|----------|--------|
| FR-001 | Issue list with search/filter via gh CLI | P0 | Implemented |
| FR-002 | Issue detail with markdown rendering | P0 | Implemented |
| FR-003 | Issue-branch linkage display | P0 | Implemented |
| FR-004 | Git View tab in management panel: file status (S/U/?), file diffs (lazy, max 50 lines), last 5 commits, divergence, PR link | P1 | Implemented |
| FR-005 | PR dashboard as independent management tab: PR list, CI check status, merge state, review status | P1 | Implemented |
| FR-006 | PR detail with CI check badges | P1 | Implemented |
| FR-007 | PR status from gwt-core::git::pr_status (PrStatus struct) | P1 | Implemented |
| FR-008 | GraphQL primary, REST fallback for GitHub API | P1 | Partially Implemented |

## Non-Functional Requirements

| ID | Requirement |
|----|-------------|
| NFR-001 | Issue search completes under 1 second |
| NFR-002 | Git View diffs are lazy-loaded (not fetched all at once) |

## Design Notes

- Git View is modeled after the old TUI v6.30.3 Git View screen (`screens/git_view.rs`)
- PR dashboard uses the existing `gwt-core::git::pr_status` module and `PrStatus` struct
- GitHub API calls prefer GraphQL for efficiency; REST is used as fallback when GraphQL is unavailable

## Success Criteria

1. Git View tab displays file status, diffs, commits, and divergence for the current branch
2. PR dashboard tab lists PRs with CI status, merge state, and review status
3. All existing Issue functionality (US-1 through US-3, US-6) continues to work without regression
4. NFR-001 and NFR-002 performance targets are met
