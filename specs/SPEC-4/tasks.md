# SPEC-4: Tasks

## Phase 1: Git View Tab Restoration

### 1.1 Git View Data Model [P]

- [ ] TEST: Unit test for `GitStatus` struct parsing `git status --porcelain` output
- [ ] TEST: Unit test for `DivergenceInfo` (ahead/behind counts)
- [ ] IMPL: Add `GitStatus`, `FileEntry`, `DivergenceInfo` structs in gwt-core
- [ ] IMPL: Add `git_file_status()` function parsing porcelain output
- [ ] IMPL: Add `git_divergence()` function parsing rev-list output

### 1.2 Diff Lazy Loading [P]

- [ ] TEST: Unit test for diff fetching with line truncation (max 50 lines)
- [ ] IMPL: Add `git_diff_file(path, staged: bool, max_lines: usize)` function
- [ ] IMPL: Diff result struct with truncation indicator

### 1.3 Commit History

- [ ] TEST: Unit test for parsing `git log --oneline -5` output
- [ ] IMPL: Add `recent_commits(count: usize)` function returning `Vec<CommitEntry>`

### 1.4 Git View Screen

- [ ] TEST: Snapshot test for Git View screen layout (file list + diff + commits)
- [ ] IMPL: Create `screens/git_view.rs` with `GitViewScreen`
  - File: `crates/gwt-tui/src/screens/git_view.rs`
- [ ] IMPL: File list widget with [S]/[U]/[?] markers
- [ ] IMPL: Diff pane with syntax-aware rendering
- [ ] IMPL: Commit history pane
- [ ] IMPL: Divergence status in header
- [ ] IMPL: PR link display when PR exists

### 1.5 Tab Registration

- [ ] IMPL: Register Git View tab in management panel tab bar
  - File: `crates/gwt-tui/src/app.rs`
- [ ] TEST: Integration test verifying Git View tab appears and responds to navigation

## Phase 2: PR Dashboard Tab

### 2.1 PR Data Model [P]

- [ ] TEST: Unit test for `PrListItem` struct construction from gh CLI output
- [ ] TEST: Unit test for CI check status mapping to display icons
- [ ] IMPL: Add `PrListItem` struct (title, number, state, ci_status, merge_state, review_status)
- [ ] IMPL: Add `fetch_pr_list()` using gh CLI (GraphQL primary, REST fallback)

### 2.2 PR Detail Data [P]

- [ ] TEST: Unit test for PR detail parsing including CI check badges
- [ ] IMPL: Add `PrDetail` struct with checks, reviews, merge readiness
- [ ] IMPL: Add `fetch_pr_detail(number: u64)` function

### 2.3 PR Dashboard Screen

- [ ] TEST: Snapshot test for PR dashboard layout (list + detail)
- [ ] IMPL: Create `screens/pr_dashboard.rs` with `PrDashboardScreen`
  - File: `crates/gwt-tui/src/screens/pr_dashboard.rs`
- [ ] IMPL: PR list widget with CI status icons
- [ ] IMPL: PR detail pane with check badges and review status
- [ ] IMPL: Auto-refresh on tab focus

### 2.4 Tab Registration

- [ ] IMPL: Register PR Dashboard tab in management panel tab bar
  - File: `crates/gwt-tui/src/app.rs`
- [ ] TEST: Integration test verifying PR Dashboard tab appears and responds to navigation

## Phase 3: Integration Testing

- [ ] TEST: End-to-end test: Git View shows correct file status for a test repo
- [ ] TEST: End-to-end test: PR dashboard shows PRs from a mock gh CLI response
- [ ] TEST: Regression test: existing Issue tab functionality unaffected
