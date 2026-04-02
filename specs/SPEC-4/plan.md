# SPEC-4: Implementation Plan

## Phase 1: Git View Tab Restoration

**Goal:** Restore the Git View tab in the management panel, modeled after old TUI v6.30.3.

### Approach

- Reference `screens/git_view.rs` from the old TUI codebase for layout and behavior
- Add a new `GitViewScreen` in `crates/gwt-tui/src/screens/`
- Register Git View as a management panel tab

### Components

1. **File status list** — Run `git status --porcelain` and parse into Staged/Unstaged/Untracked categories
2. **Diff viewer** — Lazy-load diff for selected file using `git diff` (staged) or `git diff --cached` (unstaged), truncated to 50 lines initially
3. **Commit history** — Show last 5 commits via `git log --oneline -5`
4. **Divergence status** — Parse `git rev-list --left-right --count HEAD...@{upstream}`
5. **PR link** — Use `gwt-core::git::pr_status` to check if a PR exists for the current branch

### Key Decisions

- Lazy-loading diffs prevents performance issues on large changesets
- File status uses porcelain format for stable parsing

## Phase 2: PR Dashboard Tab

**Goal:** Add a PR dashboard as an independent management panel tab.

### Approach

- Add a new `PrDashboardScreen` in `crates/gwt-tui/src/screens/`
- Use `gwt-core::git::pr_status` (PrStatus struct) for data
- Integrate `gh` CLI for PR list and CI check queries

### Components

1. **PR list** — Title, number, state, CI status icon, merge state
2. **PR detail** — CI check badges, review status, merge readiness
3. **CI check badges** — Map check conclusion to Unicode symbols
4. **Review status** — Show approved/changes-requested/pending

### API Strategy

- GraphQL primary (`gh api graphql`) for batched PR + check data
- REST fallback (`gh api repos/{owner}/{repo}/pulls`) when GraphQL is unavailable

## Dependencies

- `gwt-core::git::pr_status` module (PrStatus struct) — exists
- `gh` CLI — required for GitHub API access
- Old TUI `screens/git_view.rs` — reference for Git View layout
