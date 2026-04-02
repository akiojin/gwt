> **Canonical Boundary**: `SPEC-1787` は workspace initialization と SPEC-first workflow の正本である。gwt-spec workflow / storage / completion gate の正本は `SPEC-1579` である。

# ワークスペース初期化と SPEC 駆動ワークフロー

## Background

gwt-tui currently has several workflow gaps:

1. **Empty screen on non-repo launch** — No Clone Wizard auto-launch, leaving users with no clear next action
2. **Unnecessary Bare Clone complexity** — Normal Clone achieves the same functionality with less complexity
3. **Branch-first UX and SPEC-first workflow are misaligned** — the rebuilt TUI wants `Branches` as the primary entry, while SPEC/Issue-driven launch must remain first-class
4. **No place for SPEC drafting** — Creating SPECs requires a branch, but without SPECs there's no reason to create a branch (chicken-and-egg problem)
5. **No develop commit protection** — When agents run on develop for brainstorming, accidental commits risk polluting the protected branch

This SPEC replaces SPEC-1647 (Project Management, draft) which was outdated and never fully specified.

### References

- SPEC-1647 (Project Management) — superseded by this SPEC
- SPEC-1776 (TUI Migration) — parent TUI architecture
- SPEC-1654 (Workspace Shell) — tab/session management
- SPEC-1644 (Local Git Backend) — ref/worktree semantics

### Design Document

Plan file: `/Users/akiojin/.claude/plans/cozy-spinning-cookie.md`

## User Stories

### User Story 1 - First-time workspace setup (Priority: P0)

As a developer launching gwt-tui in a directory without a Git repository, I want to be guided through repository cloning so that I can start working immediately.

**Acceptance Scenarios:**

1. **Given** gwt-tui is launched in an empty directory, **When** the TUI starts, **Then** a fullscreen initialization screen is displayed (modal, blocking other navigation)
2. **Given** the initialization screen is displayed, **When** the user enters a repository URL and presses Enter, **Then** a Normal Shallow Clone (`git clone --depth=1 -b develop`) is executed
3. **Given** the clone completes successfully, **When** the TUI transitions, **Then** the repo_root is switched in-place (no restart) and the rebuilt primary entry is shown without losing access to `SPECs` as a first-class tab
4. **Given** the clone fails (network error, auth failure, invalid URL), **When** the error is shown, **Then** the user is returned to the URL input step to retry
5. **Given** the initialization screen is displayed, **When** the user presses Esc, **Then** the TUI exits
6. **Given** gwt-tui is launched in a non-empty non-repo directory, **When** the TUI starts, **Then** the same initialization screen is displayed (same behavior as empty directory)

### User Story 2 - Abolish Bare Clone, adopt Normal Clone (Priority: P0)

As a developer, I want gwt to use Normal Clone instead of Bare Clone so that the repository structure is simpler and I can work on develop immediately.

**Acceptance Scenarios:**

1. **Given** the Clone Wizard is invoked, **When** the user provides a URL, **Then** only Normal Shallow Clone is offered (no Bare/BareShallow selection)
2. **Given** the clone target has a `develop` branch, **When** cloning completes, **Then** `develop` is checked out
3. **Given** the clone target does not have a `develop` branch, **When** cloning completes, **Then** the default branch (e.g., main) is checked out
4. **Given** an existing Bare repository, **When** gwt-tui is launched, **Then** an error message is displayed explaining that Bare repositories are no longer supported, with instructions to re-clone using Normal Clone (`git clone --depth=1 -b develop <url>`)

### User Story 3 - SPEC/Issue-driven agent launch (Priority: P0)

As a developer, I want to launch agents from the SPECs or Issues tab as first-class entry points even though `Branches` remains the rebuilt shell's primary entry, so that work stays traceable to a specification.

**Acceptance Scenarios:**

1. **Given** I select a SPEC in the SPECs tab, **When** I press the Launch Agent key, **Then** the agent launch wizard opens with `feature/feature-{N}` pre-filled as the branch name
2. **Given** I select an Issue in the Issues tab, **When** I press the Launch Agent key, **Then** the agent launch wizard opens with an appropriate branch name derived from the Issue
3. **Given** the SPECs tab is empty, **When** the tab is displayed, **Then** a guide message is shown (e.g., "No SPECs yet. Press [n] to create one.")
4. **Given** I launch an agent from Branches tab without SPEC/Issue association, **When** the agent starts, **Then** it runs without association (ad-hoc work is permitted)

### User Story 4 - SPEC drafting workflow on develop (Priority: P0)

As a developer, I want to brainstorm with an agent on the develop branch and have it create SPECs with proper feature branches so that I don't need to manually manage branches for SPEC creation.

**Acceptance Scenarios:**

1. **Given** I press "New SPEC" in the SPECs tab, **When** the action triggers, **Then** an agent is launched on the develop branch with the SPEC drafting skill
2. **Given** the agent determines the work should be split into multiple SPECs, **When** each SPEC is finalized, **Then** the agent creates `feature/feature-{N}` for each, commits SPEC files, and returns to develop
3. **Given** all SPECs are registered, **When** the agent is ready to implement, **Then** it proceeds to implement the highest-priority SPEC on its feature branch
4. **Given** an agent is running on develop, **When** it attempts to commit, **Then** the pre-commit hook blocks the commit with a clear error message

### User Story 5 - develop branch commit protection (Priority: P0)

As a project maintainer, I want develop to be protected from accidental direct commits so that the branch remains clean for PR merges only.

**Acceptance Scenarios:**

1. **Given** gwt clones a new repository, **When** clone completes, **Then** a pre-commit hook is automatically installed that blocks commits on develop
2. **Given** the pre-commit hook is installed, **When** any process attempts `git commit` on develop, **Then** the commit is rejected with an error message indicating develop is protected
3. **Given** AGENTS.md, **When** an agent reads its instructions, **Then** it finds a rule prohibiting direct commits to develop

## Edge Cases

- Clone URL with authentication required (SSH key, token) — Clone Wizard should display the git error message clearly
- Repository with no branches at all — Clone should succeed with empty repo, initialization screen should handle gracefully
- Interrupted clone (Ctrl+C during clone) — Partial clone directory should be cleaned up
- Pre-commit hook already exists — gwt should append/merge rather than overwrite existing hooks
- Agent on develop creates files but doesn't commit — Uncommitted changes carry over to feature branch via `git checkout -b`, which is acceptable behavior

## Functional Requirements

- FR-001: Non-repo detection must use existing `detect_repo_type()` returning `NonRepo` or `Empty`
- FR-002: Initialization screen must be fullscreen and modal (block Ctrl+G layer switching)
- FR-003: Clone must use `git clone --depth=1 -b develop <url>`, falling back to `--depth=1` if develop doesn't exist
- FR-004: After clone, `Model::reset(new_repo_root)` must reload all screen data in-place
- FR-005: `Model::reset()` must transition to the rebuilt management entry without losing first-class access to `SPECs`
- FR-006: Bare Clone support must be removed from Clone Wizard (CloneType enum)
- FR-007: `RepoType::Bare`, `find_bare_repo_in_dir()`, `is_bare_repository()` must be deleted
- FR-008: `BareProjectConfig` (`bare_project.rs`) must be deleted
- FR-009: Worktree Sibling location strategy (Bare-specific) must be removed
- FR-010: `resolve_repo_root()` must remove Bare detection fallback
- FR-011: Bare migration dialog must be removed
- FR-012: SPECs tab must show guide message when `specs/` is empty
- FR-013: SPECs tab must support "Launch Agent" action with auto-derived `feature/feature-{N}` branch name
- FR-014: Issues tab must support "Launch Agent" action with auto-derived branch name
- FR-015: Wizard must support SPEC/Issue-origin launch with pre-filled branch name
- FR-016: Wizard must support "SPEC drafting" mode launching agent on develop with SPEC drafting skill context
- FR-017: A new SPEC drafting skill must be created integrating gwt-spec-register/clarify/plan/tasks with brainstorming phase and branch creation
- FR-018: `AGENTS.md` must include develop direct-commit prohibition rule
- FR-019: Pre-commit hook must be auto-installed after clone and on first launch in existing repos
- FR-020: Pre-commit hook must block commits on develop branch with clear error message
- FR-021: Data load logic in `app.rs` must be extracted to `Model::load_all_data()` for reuse

## Non-Functional Requirements

- NFR-001: Clone operation must show progress indication (existing clone_wizard progress display)
- NFR-002: `Model::reset()` must complete within 2 seconds for typical repositories
- NFR-003: Pre-commit hook must not interfere with existing hooks in `.git/hooks/`

## Success Criteria

- SC-001: gwt-tui launched in empty directory shows initialization screen, not empty Branches tab
- SC-002: Clone Wizard offers only Normal Shallow Clone (no Bare options)
- SC-003: After clone, TUI displays SPECs tab without restart
- SC-004: "New SPEC" from SPECs tab launches agent on develop with SPEC drafting skill
- SC-005: Agent creates feature/feature-{N} branches for each SPEC and commits SPEC files
- SC-006: `git commit` on develop is blocked by pre-commit hook
- SC-007: All Bare-related code is removed, `cargo build` succeeds without Bare references
- SC-008: `cargo test -p gwt-core -p gwt-tui` passes
- SC-009: `cargo clippy --all-targets --all-features -- -D warnings` passes
