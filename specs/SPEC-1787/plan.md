# Plan: SPEC-1787 — ワークスペース初期化と SPEC 駆動ワークフロー

## Summary

Replace Bare Clone with Normal Clone, add non-repo initialization flow, keep SPEC/Issue-driven launch first-class within a branch-first rebuilt shell, create SPEC drafting skill, and protect develop branch from direct commits.

## Technical Context

### Affected Modules

| Module | Files | Change Type |
|--------|-------|-------------|
| gwt-core/git | `repository.rs` | Remove Bare detection (RepoType::Bare, is_bare_repository, find_bare_repo_in_dir) |
| gwt-core/config | `bare_project.rs`, `mod.rs` | Delete file, remove re-export |
| gwt-core/worktree | `manager.rs` | Remove Sibling strategy |
| gwt-core/git | `hooks.rs` (new) | Pre-commit hook install |
| gwt-tui/model | `model.rs` | Add ActiveLayer::Initialization, Model::reset() |
| gwt-tui/app | `app.rs` | Initialization layer handling, load_all_data() extraction, rebuilt primary-entry handoff |
| gwt-tui/screens | `clone_wizard.rs` | Normal Clone only, fullscreen mode |
| gwt-tui/screens | `specs.rs` | Launch Agent, New SPEC, guide message |
| gwt-tui/screens | `issues.rs` | Launch Agent action |
| gwt-tui/screens | `wizard.rs` | SPEC/Issue-origin launch, SPEC drafting mode |
| gwt-tui/screens | `migration_dialog.rs` | Delete file |
| gwt-tui | `main.rs` | Remove Bare fallback from resolve_repo_root() |
| project root | `AGENTS.md` | develop commit prohibition rule |
| claude skills | `gwt-spec-draft/SKILL.md` (new) | SPEC drafting skill |

### Assumptions

- Normal Clone repositories use `.worktrees/` subdirectory strategy for any future worktree creation
- `git clone --depth=1 -b develop` fails gracefully if develop doesn't exist (git returns error, we retry without `-b develop`)
- Pre-commit hook is a bash script checking `git symbolic-ref HEAD` against `refs/heads/develop`
- Existing Bare repositories show an error message and do not attempt automatic migration

### Constraints from Constitution

- §1 Spec Before Implementation: This plan covers the full artifact set before code
- §2 Test-First: Each phase has corresponding test requirements
- §4 Minimal Complexity: Normal Clone is simpler than Bare Clone — this change reduces complexity
- §6 SPEC Category: **CORE-TUI** (touches TUI initialization, git operations, and workflow)

## Constitution Check

| Rule | Status | Notes |
|------|--------|-------|
| §1 Spec Before Implementation | ✅ PASS | spec.md complete, plan.md in progress |
| §2 Test-First Delivery | ✅ PASS | Test plan defined per phase |
| §3 No Workaround-First | ✅ PASS | Root cause addressed (Bare complexity removed, not wrapped) |
| §4 Minimal Complexity | ✅ PASS | Net reduction in code (Bare removal). New features are minimal additions |
| §5 Verifiable Completion | ✅ PASS | SC-001 through SC-009 are all verifiable |
| §6 SPEC vs Issue | ✅ PASS | Category: CORE-TUI. 5 user stories, decomposable into ~12 tasks |

## Complexity Tracking

| Addition | Justification |
|----------|--------------|
| `ActiveLayer::Initialization` | Required for modal fullscreen init screen. Single enum variant, minimal |
| `Model::reset()` | Required for in-place repo_root switch. Reuses existing load functions |
| Pre-commit hook install | Required for develop protection. Simple bash script, ~10 lines |
| SPEC drafting skill | Required for brainstorming → SPEC creation flow. Markdown skill file only |

## Project Structure

```
crates/
├── gwt-core/src/
│   ├── config/
│   │   ├── bare_project.rs     ← DELETE
│   │   └── mod.rs              ← remove bare_project re-export
│   ├── git/
│   │   ├── repository.rs       ← remove Bare detection
│   │   └── hooks.rs            ← NEW: pre-commit hook install
│   └── worktree/
│       └── manager.rs          ← remove Sibling strategy
├── gwt-tui/src/
│   ├── main.rs                 ← simplify resolve_repo_root()
│   ├── model.rs                ← add Initialization layer, reset()
│   ├── app.rs                  ← init screen handling, load_all_data()
│   └── screens/
│       ├── clone_wizard.rs     ← Normal Clone only, fullscreen mode
│       ├── migration_dialog.rs ← DELETE
│       ├── specs.rs            ← Launch Agent, New SPEC, guide message
│       ├── issues.rs           ← Launch Agent action
│       └── wizard.rs           ← SPEC/Issue-origin, SPEC drafting mode
├── AGENTS.md                   ← develop commit prohibition
└── .claude/skills/
    └── gwt-spec-draft/
        └── SKILL.md            ← NEW: SPEC drafting skill
```

## Phased Implementation

### Phase 1: Bare Abolition (FR-006 through FR-011)

**Goal**: Remove all Bare Clone code paths. Net code reduction.

**Files**:
1. `crates/gwt-core/src/git/repository.rs` (lines 41-52, 112-122, 135-154, 157-179)
   - Remove `RepoType::Bare` variant
   - Delete `is_bare_repository()` function
   - Delete `find_bare_repo_in_dir()` function
   - Update `detect_repo_type()` to remove Bare detection step
2. `crates/gwt-core/src/config/bare_project.rs` — delete entire file
3. `crates/gwt-core/src/config/mod.rs` (line 23) — remove `bare_project` re-export
4. `crates/gwt-core/src/worktree/manager.rs` (lines 43-52)
   - Remove Bare detection in `WorktreeManager::new()`
   - Remove `WorktreeLocation::Sibling` branch
5. `crates/gwt-tui/src/main.rs` (lines 27-36)
   - Simplify `resolve_repo_root()`: remove `find_bare_repo_in_dir` fallback
6. `crates/gwt-tui/src/screens/migration_dialog.rs` — delete entire file
7. `crates/gwt-tui/src/screens/clone_wizard.rs` (lines 29-34)
   - Remove `CloneType::Bare` and `CloneType::BareShallow` from enum
   - Remove `CloneStep::TypeSelect` step
   - Change clone command to `git clone --depth=1 -b develop <url>`
8. Fix all compilation errors from removed types (grep for `Bare`, `BareProjectConfig`, `MigrationDialog`, `Sibling`)

**Verification**: `cargo build -p gwt-core -p gwt-tui`, `cargo test`, `cargo clippy`

### Phase 2: Initialization Flow (FR-001, FR-002, FR-003, FR-004, FR-005, FR-021)

**Goal**: Non-repo directories show fullscreen initialization screen, clone completes with in-place switch, and the rebuilt shell lands on the branch-first primary entry while preserving first-class `SPECs` access.

**Files**:
1. `crates/gwt-tui/src/model.rs` (lines 29-32, 168-256)
   - Add `ActiveLayer::Initialization` to enum
   - Add `Model::reset(new_repo_root: PathBuf)` method
   - In `Model::new()`, detect repo type and set Initialization layer if NonRepo/Empty
2. `crates/gwt-tui/src/app.rs` (lines 2045-2086)
   - Extract data loading into `Model::load_all_data(&mut self, repo_root: &Path)`
   - Add `ActiveLayer::Initialization` draw/update branches
   - On clone complete: call `model.reset(cloned_path)`, transition to SPECs tab
   - Block Ctrl+G during Initialization (modal)
   - Esc during Initialization → exit TUI
3. `crates/gwt-tui/src/screens/clone_wizard.rs` (line 173)
   - Add `render_fullscreen()` variant alongside existing `render_clone_wizard()`
   - Fullscreen draws URL input centered in terminal, no overlay border

**Verification**: Launch gwt-tui from empty dir → init screen appears. Clone → SPECs tab.

### Phase 3: develop Commit Protection (FR-018, FR-019, FR-020)

**Goal**: Prevent accidental commits to develop via hook + prompt.

**Files**:
1. `crates/gwt-core/src/git/hooks.rs` (new file)
   - `pub fn install_pre_commit_hook(repo_root: &Path) -> io::Result<()>`
   - Hook script: check branch, block if develop, pass through otherwise
   - Merge-safe: if `.git/hooks/pre-commit` exists, append gwt guard section
2. `crates/gwt-tui/src/app.rs`
   - Call `install_pre_commit_hook()` after clone completes (Phase 2 integration)
   - Call on first launch if hook not yet installed
3. `AGENTS.md`
   - Add rule: "develop ブランチへの直接コミットは禁止"

**Verification**: `git commit` on develop → blocked. `git commit` on feature → allowed.

### Phase 4: SPEC/Issue Launch Actions (FR-012, FR-013, FR-014, FR-015)

**Goal**: SPECs/Issues tabs can launch agents with auto-derived branch names.

**Files**:
1. `crates/gwt-tui/src/screens/specs.rs` (lines 76-93, 95-150, 247)
   - Add guide message when specs list is empty: "No SPECs yet. Press [n] to create one."
   - Enhance existing `LaunchAgent` message to derive `feature/feature-{N}` from SPEC ID
2. `crates/gwt-tui/src/screens/issues.rs`
   - Add `LaunchAgent` message and key binding
   - Derive branch name from Issue number (e.g., `feature/issue-{N}`)
3. `crates/gwt-tui/src/screens/wizard.rs` (lines 271-350, 428-450)
   - Extend `open_for_spec()` to set `is_new_branch = true` and pre-fill branch name
   - Add `open_for_issue()` method with similar logic
   - Support `git checkout -b` from develop when launching from SPEC/Issue

**Verification**: Select SPEC → Launch Agent → wizard opens with correct branch name.

### Phase 5: SPEC Drafting Skill + Workflow (FR-016, FR-017)

**Goal**: "New SPEC" action launches agent on develop with SPEC drafting skill.

**Files**:
1. `.claude/skills/gwt-spec-draft/SKILL.md` (new file)
   - Integrates gwt-spec-register/clarify/plan/tasks workflow
   - Adds brainstorming phase (user interview → requirements analysis)
   - Handles multiple SPEC creation (each with feature/feature-{N} branch)
   - Returns to develop between SPEC creations
2. `crates/gwt-tui/src/screens/specs.rs`
   - Add "New SPEC" action (e.g., `n` key)
   - Trigger agent launch on develop with SPEC drafting skill context
3. `crates/gwt-tui/src/screens/wizard.rs`
   - Add `open_for_spec_drafting()` method
   - Sets working directory to repo root (develop checkout)
   - Injects SPEC drafting skill path into agent launch context

**Verification**: Press `n` in SPECs tab → agent launches on develop → creates SPEC → commits to feature branch.

## Risk Assessment

| Risk | Mitigation |
|------|-----------|
| Bare removal breaks existing users | Error message with re-clone instructions (US2-AS4) |
| Clone fails for repos without develop | Fallback to default branch (FR-003) |
| Pre-commit hook conflicts with existing hooks | Append gwt section, don't overwrite (Edge Case) |
| Agent commits to develop despite protections | Double defense: hook blocks + AGENTS.md prohibition |
| Model::reset() leaves stale state | Clear all session tabs and reload all data |

## Acceptance Scenario Verification

| SC | Verification Method |
|----|-------------------|
| SC-001 | Integration test: launch from empty dir, assert Initialization layer |
| SC-002 | Unit test: CloneType enum has no Bare variants |
| SC-003 | Integration test: clone completes, assert ManagementTab::Specs active |
| SC-004 | Manual test: press `n` in SPECs tab, verify agent on develop |
| SC-005 | Manual test: agent creates branch, commits SPEC files |
| SC-006 | Unit test: pre-commit hook script blocks develop commits |
| SC-007 | Compile test: `cargo build` succeeds, grep for "Bare" returns 0 |
| SC-008 | `cargo test -p gwt-core -p gwt-tui` |
| SC-009 | `cargo clippy --all-targets --all-features -- -D warnings` |
