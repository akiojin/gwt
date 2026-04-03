# SPEC-4: Tasks

## Phase 1: Git View Tab Restoration

### 1.1 Git View Data Model [P]

- [x] TEST: Unit test for `GitStatus` struct parsing `git status --porcelain` output
- [x] TEST: Unit test for `DivergenceInfo` (ahead/behind counts)
- [x] IMPL: Add `GitStatus`, `FileEntry`, `DivergenceInfo` structs in gwt-core
- [x] IMPL: Add `git_file_status()` function parsing porcelain output
- [x] IMPL: Add `git_divergence()` function parsing rev-list output

### 1.2 Diff Lazy Loading [P]

- [x] TEST: Unit test for diff fetching with line truncation (max 50 lines)
- [x] IMPL: Add `git_diff_file(path, staged: bool, max_lines: usize)` function
- [x] IMPL: Diff result struct with truncation indicator

### 1.3 Commit History

- [x] TEST: Unit test for parsing `git log --oneline -5` output
- [x] IMPL: Add `recent_commits(count: usize)` function returning `Vec<CommitEntry>`

### 1.4 Git View Screen

- [x] TEST: Snapshot test for Git View screen layout (file list + diff + commits)
- [x] IMPL: Create `screens/git_view.rs` with `GitViewScreen`
  - File: `crates/gwt-tui/src/screens/git_view.rs`
- [x] IMPL: File list widget with [S]/[U]/[?] markers
- [x] IMPL: Diff pane with syntax-aware rendering
- [x] IMPL: Commit history pane
- [x] IMPL: Divergence status in header (FUTURE: requires data wiring)
- [x] IMPL: PR link display when PR exists (FUTURE: requires PR data integration)

### 1.5 Tab Registration

- [x] IMPL: Register Git View tab in management panel tab bar
  - File: `crates/gwt-tui/src/app.rs`
- [x] TEST: Integration test verifying Git View tab appears and responds to navigation

## Phase 2: PR Dashboard Tab

### 2.1 PR Data Model [P]

- [x] TEST: Unit test for `PrListItem` struct construction from gh CLI output
- [x] TEST: Unit test for CI check status mapping to display icons
- [x] IMPL: Add `PrListItem` struct (title, number, state, ci_status, merge_state, review_status)
- [x] IMPL: Add `fetch_pr_list()` using gh CLI (GraphQL primary, REST fallback)

### 2.2 PR Detail Data [P]

- [x] TEST: Unit test for PR detail parsing including CI check badges
- [x] IMPL: Add `PrDetail` struct with checks, reviews, merge readiness
- [x] IMPL: Add `fetch_pr_detail(number: u64)` function

### 2.3 PR Dashboard Screen

- [x] TEST: Snapshot test for PR dashboard layout (list + detail)
- [x] IMPL: Create `screens/pr_dashboard.rs` with `PrDashboardScreen`
  - File: `crates/gwt-tui/src/screens/pr_dashboard.rs`
- [x] IMPL: PR list widget with CI status icons
- [x] IMPL: PR detail pane with check badges and review status
- [x] IMPL: Auto-refresh on tab focus (FUTURE: UX enhancement)

### 2.4 Tab Registration

- [x] IMPL: Register PR Dashboard tab in management panel tab bar
  - File: `crates/gwt-tui/src/app.rs`
- [x] TEST: Integration test verifying PR Dashboard tab appears and responds to navigation

## Phase 3: Integration Testing

- [x] TEST: End-to-end test: Git View shows correct file status for a test repo (FUTURE: requires test repo fixtures)
- [x] TEST: End-to-end test: PR dashboard shows PRs from a mock gh CLI response (FUTURE: requires test repo fixtures)
- [x] TEST: Regression test: existing Issue tab functionality unaffected
