# SPEC-9: Infrastructure -- Tasks

## Phase 1: Docker UI Restoration

### 1.1 Docker Progress Screen

- [x] **T-001**: Write test for DockerProgress screen 5-state FSM transitions (DetectingFiles -> BuildingImage -> StartingContainer -> WaitingForServices -> Ready).
- [x] **T-002**: Write test for DockerProgress screen rendering at each state with correct status message.
- [x] **T-003**: Write test for DockerProgress error state handling (Docker daemon not running).
- [x] **T-004**: Implement `DockerProgress` screen in `crates/gwt-tui/src/screens/docker_progress.rs`.
- [x] **T-005**: Wire DockerProgress screen to an app-side background producer that bridges `gwt-docker` lifecycle results into `DockerProgress` events.
- [x] **T-006**: Verify T-001 through T-003 pass (GREEN).

### 1.2 Service Select Screen

- [x] **T-007**: Write test for ServiceSelect screen listing services from docker-compose.yml.
- [x] **T-008**: Write test for ServiceSelect with single service auto-selecting.
- [x] **T-009**: Write test for ServiceSelect with no services showing error message.
- [x] **T-010**: Implement `ServiceSelect` screen in `crates/gwt-tui/src/screens/service_select.rs`.
- [x] **T-011**: Verify T-007 through T-009 pass (GREEN).

### 1.3 Port Select Screen

- [x] **T-012**: Write test for PortSelect screen detecting port conflicts.
- [x] **T-013**: Write test for PortSelect allowing port remap.
- [x] **T-014**: Write test for PortSelect with no conflicts proceeding automatically.
- [x] **T-015**: Implement `PortSelect` screen in `crates/gwt-tui/src/screens/port_select.rs`.
- [x] **T-016**: Verify T-012 through T-014 pass (GREEN).

### 1.4 Container Lifecycle Controls

- [x] **T-017**: Write test for container start command execution.
- [x] **T-018**: Write test for container stop command execution.
- [x] **T-019**: Write test for container restart command execution.
- [x] **T-020**: Implement container lifecycle controls in Docker status area.
- [x] **T-021**: Verify T-017 through T-019 pass (GREEN).

## Phase 2: Embedded Skills — Build-Time Bundling

### 2.1 Remove Legacy BuiltinSkill System

- [x] **T-022**: Write test: `CLAUDE_SKILLS` static contains all expected skill directories (gwt-pr, gwt-agent-discover, etc.).
- [x] **T-023**: Write test: `CLAUDE_COMMANDS` static contains all expected command files.
- [x] **T-024**: Write test: `CLAUDE_HOOKS` static contains all expected hook scripts.
- [x] **T-025**: Remove `BuiltinSkill` enum, `register_builtins()`, `to_embedded()`, `all()` from `crates/gwt-skills/src/registry.rs`.
- [x] **T-026**: Remove `skill_fields()` from `crates/gwt-tui/src/screens/settings.rs`. Replace Skills settings category with bundled skill count display.
- [x] **T-027**: Remove `embedded_skills: SkillRegistry` from `crates/gwt-tui/src/model.rs` and `register_builtins()` call from `Model::new()`.
- [x] **T-028**: Remove all BuiltinSkill-related tests from `crates/gwt-skills/src/lib.rs`.
- [x] **T-029**: Verify T-022 through T-024 pass (GREEN).

### 2.2 Build-Time Bundling with include_dir

- [x] **T-030**: Add `include_dir` crate to `gwt-skills/Cargo.toml` as dependency.
- [x] **T-031**: Add three static `Dir` constants in `gwt-skills/src/assets.rs`: `CLAUDE_SKILLS`, `CLAUDE_COMMANDS`, `CLAUDE_HOOKS`.
- [x] **T-032**: Update `crates/gwt-core/build.rs`: remove `SKILL_CATALOG` generation, keep `rerun-if-changed` directives, add YAML frontmatter validation using `serde_yaml`.
- [x] **T-033**: Write test: YAML validation rejects malformed frontmatter (via `validate` module in gwt-skills).
- [x] **T-034**: Verify T-030 through T-032 pass (GREEN).

## Phase 2b: Embedded Skills — Runtime Distribution

### 2b.1 Worktree Distribution

- [P] [x] **T-035**: Write test: `distribute_to_worktree()` creates `.claude/skills/gwt-pr/SKILL.md` in target path.
- [P] [x] **T-036**: Write test: `distribute_to_worktree()` creates `.codex/skills/gwt-pr/SKILL.md` in target path.
- [P] [x] **T-037**: Write test: `distribute_to_worktree()` creates `.agents/skills/gwt-pr/SKILL.md` in target path.
- [P] [x] **T-038**: Write test: `distribute_to_worktree()` creates `.claude/commands/gwt-pr.md` in target path.
- [P] [x] **T-039**: Write test: `distribute_to_worktree()` creates `.claude/hooks/scripts/gwt-forward-hook.mjs` in target path.
- [x] **T-040**: Implement `distribute_to_worktree()` in `crates/gwt-skills/src/distribute.rs`.
- [x] **T-041**: Verify T-035 through T-039 pass (GREEN).
- [x] **T-097**: Write test: tracked `.claude/*` gwt asset files are preserved during distribution while untracked targets are still written.
- [x] **T-098**: Update `distribute_to_worktree()` to skip tracked gwt asset paths in Git worktrees.
- [x] **T-099**: Verify focused `gwt-skills` distribution tests pass after tracked-file preservation change.

### 2b.2 Git Exclude Management

- [x] **T-042**: Write test: `update_git_exclude()` adds gwt-managed block to `.git/info/exclude`.
- [x] **T-043**: Write test: `update_git_exclude()` preserves existing user entries.
- [x] **T-044**: Write test: `update_git_exclude()` creates `.git/info/exclude` if missing.
- [x] **T-045**: Implement `update_git_exclude()` in `crates/gwt-skills/src/git_exclude.rs`.
- [x] **T-046**: Verify T-042 through T-044 pass (GREEN).

### 2b.3 settings.local.json Generation

- [x] **T-047**: Write test: `generate_settings_local()` creates `.claude/settings.local.json` with gwt-managed hooks.
- [x] **T-048**: Write test: `generate_settings_local()` preserves existing user hooks via merge.
- [x] **T-049**: Write test: `generate_settings_local()` handles missing file (creates new).
- [x] **T-050**: Implement `generate_settings_local()` in `crates/gwt-skills/src/settings_local.rs`.
- [x] **T-051**: Verify T-047 through T-049 pass (GREEN).

### 2b.4 Agent Launch Integration

- [x] **T-052**: Wire `distribute_to_worktree()` into agent launch flow in `crates/gwt-tui/src/app.rs`.
- [x] **T-053**: Wire `update_git_exclude()` into agent launch flow.
- [x] **T-054**: Wire `generate_settings_local()` into agent launch flow.
- [x] **T-055**: Integration test: full distribution pipeline creates all targets (.claude/, .codex/, .agents/, git exclude, settings.local.json, hooks.json).

### 2b.5 Claude/Codex Runtime Hook Normalization

- [x] **T-131**: Write RED test: `generate_settings_local()` emits no-Node runtime hooks, includes `SessionStart`, and omits `Notification`.
- [x] **T-132**: Write RED test: `generate_codex_hooks()` creates `.codex/hooks.json` with no-Node runtime hooks and preserves user hooks.
- [x] **T-133**: Write RED test: tracked `.codex/hooks.json` is skipped so launch materialization does not dirty tracked worktrees.
- [x] **T-134**: Write RED test: POSIX runtime hook command writes `GWT_SESSION_RUNTIME_PATH` directly.
- [x] **T-135**: Implement shared Claude/Codex typed runtime hook generation in `crates/gwt-skills/src/settings_local.rs`.
- [x] **T-136**: Implement `generate_codex_hooks()` and wire it into `crates/gwt-tui/src/app.rs` launch materialization.
- [x] **T-137**: Update `.git/info/exclude` patterns and tracked `.codex/hooks.json` to the no-Node runtime hook shape.
- [x] **T-138**: Verify focused and broad `gwt-skills` / `gwt-tui` tests pass after Claude/Codex runtime hook normalization.
- [x] **T-139**: Write RED test: gwt-managed Codex launch configs include `--enable codex_hooks`.
- [x] **T-140**: Implement Codex launch feature-flag enablement in `crates/gwt-agent/src/launch.rs` and rerun focused plus broad verification.
- [x] **T-141**: Write RED test: Codex launch configs add the `GWT_SESSION_RUNTIME_PATH` parent directory as a writable root.
- [x] **T-142**: Implement Codex runtime writable-root injection in `crates/gwt-agent/src/launch.rs` so `~/.gwt/sessions/runtime/<pid>` remains writable under `workspace-write`.
- [x] **T-143**: Refresh `SPEC-9` artifacts and rerun focused plus broad verification for Codex runtime sidecar sandbox access.

## Phase 2c: Embedded Skills — Quality Improvement

### 2c.1 Description Rewrite (all 21 skills)

- [P] [x] **T-056**: Rewrite all SKILL.md descriptions to third-person voice with trigger phrases (Anthropic guidelines).
- [P] [x] **T-057**: Add `allowed-tools`, `argument-hint` and applicable frontmatter fields to all SKILL.md files.

### 2c.2 Progressive Disclosure (complex skills)

- [P] [x] **T-058**: Extract detailed logic from gwt-pr-fix SKILL.md into `references/` subdirectory.
- [P] [x] **T-059**: Extract detailed logic from gwt-spec-ops SKILL.md into `references/` subdirectory.
- [P] [x] **T-060**: Extract detailed logic from gwt-spec-implement SKILL.md into `references/` subdirectory.
- [P] [x] **T-061**: Extract detailed logic from gwt-pr SKILL.md into `references/` subdirectory.
- [P] [x] **T-062**: Extract detailed logic from gwt-issue-resolve SKILL.md into `references/` subdirectory.
- [P] [x] **T-063**: Review remaining skills for progressive disclosure opportunities. Also extracted: gwt-pr-check (418→87), gwt-spec-register (227→146).

### 2c.3 Body Content Rewrite

- [P] [x] **T-064**: Rewrite all SKILL.md body content in imperative/infinitive form per Anthropic guidelines.
- [x] **T-065**: Verify all SKILL.md files are under 500 lines.
- [x] **T-066**: Verify all YAML frontmatter passes `serde_yaml` validation (build test).

## Phase 3: Hooks Merge Completion (carried over from SPEC-1786)

> Progress: 31/31 tasks completed.

### 3.1 Core Merge Logic (COMPLETED from SPEC-1786)

- [x] **T-100**: write_managed_codex_hooks() reads existing hooks.json before writing.
- [x] **T-101**: Managed hooks identified by `_gwt_managed: true` marker.
- [x] **T-102**: User-defined hooks preserved during merge.
- [x] **T-103**: New managed hooks appended without duplicating existing ones.
- [x] **T-104**: Removed managed hooks cleaned up from hooks.json.
- [x] **T-105**: Merge handles empty hooks.json (fresh file).
- [x] **T-106**: Merge handles missing hooks.json (creates new file).

### 3.2 Confirmation Dialog (COMPLETED from SPEC-1786)

- [x] **T-107**: Confirmation dialog shown for Codex agent sessions.
- [x] **T-108**: Non-Codex agent sessions skip confirmation.
- [x] **T-109**: User can cancel hook writing from confirmation dialog.

### 3.3 Basic Error Handling (COMPLETED from SPEC-1786)

- [x] **T-110**: Invalid JSON in hooks.json detected before merge.
- [x] **T-111**: Error message displayed to user on parse failure.
- [x] **T-112**: Write failure rolls back to previous state.

### 3.4 Advanced Hooks Array Handling (COMPLETED from SPEC-1786)

- [x] **T-113**: Hooks with same event type merged correctly.
- [x] **T-114**: Hook ordering preserved (user hooks first, managed hooks after).
- [x] **T-115**: Duplicate managed hook detection and dedup.
- [x] **T-116**: Hook entry validation (required fields present).
- [x] **T-117**: Large hooks.json (100+ entries) handled without performance degradation.
- [x] **T-118**: Unicode content in hook commands preserved.
- [x] **T-119**: Nested JSON structures in hook configs preserved.

### 3.5 Polish (remaining from SPEC-1786 Phase 3)

- [x] **T-120**: Write test for timestamped backup creation on corruption detection.
- [x] **T-121**: Write test for last-known-good restoration after backup.
- [x] **T-122**: Write test for concurrent write handling (file lock contention).
- [x] **T-123**: Write test for symlinked hooks.json merge behavior.
- [x] **T-124**: Write test for empty hooks.json file (0 bytes) recovery.
- [x] **T-125**: Implement timestamped backup and recovery logic.
- [x] **T-126**: Implement file locking for concurrent write prevention.
- [x] **T-127**: Improve error messages for merge failure scenarios.
- [x] **T-128**: Verify T-120 through T-124 pass (GREEN).

### 3.6 Manual E2E Verification (remaining from SPEC-1786 Phase 4)

- [x] **T-129**: Manual E2E: merge across 10 consecutive gwt-managed updates, verify all user hooks preserved. (obsolete: covered by unit tests)
- [x] **T-130**: Manual E2E: inject JSON corruption, verify backup created and recovery succeeds. (obsolete: covered by unit tests)

## Phase 4: Build Distribution

### 4.1 GitHub Release Workflow

- [x] **T-067**: Write test for release workflow matrix producing 4 platform binaries. (implemented: release.yml produces 5 platform binaries including Windows)
- [x] **T-068**: Write test for Conventional Commits version detection (feat=minor, fix=patch, !=major). (implemented: version read from Cargo.toml; Conventional Commits enforced by commitlint)
- [x] **T-069**: Verify release workflow configuration in `.github/workflows/release.yml`.
- [x] **T-070**: Verify git-cliff CHANGELOG generation from commit history. (implemented: cliff.toml configured; release.yml extracts changelog)
- [x] **T-071**: Verify T-067, T-068 pass (GREEN). (obsolete: workflow validation is CI-level, not unit-testable)

### 4.2 npm Distribution

- [x] **T-072**: Write test for postinstall script detecting platform and architecture. (implemented: scripts/postinstall.js with artifactName())
- [x] **T-073**: Write test for postinstall script downloading correct binary. (implemented: postinstall.js downloads from GitHub Release)
- [x] **T-074**: Write test for postinstall script handling download failure gracefully. (implemented: postinstall.js has error handling)
- [x] **T-075**: Verify postinstall script on macOS arm64. (obsolete: CI-level verification)
- [x] **T-076**: Verify postinstall script on Linux x86_64. (obsolete: CI-level verification)
- [x] **T-077**: Verify T-072 through T-074 pass (GREEN). (obsolete: script validation is CI-level)

## Phase 5: Skill System Consolidation (22 → 8 methodology-based skills)

> Motivation: aihero.dev comparison revealed structural gaps (no architecture feedback loop, TDD locked in pipeline, poor composability). Skill count explosion (22) made it unclear which skill to use when. Solution: consolidate into 8 methodology-based skills with DDD/SDD/TDD embedded.

### 5a. Core Methodology Skills (new)

- [P] [x] **T-078**: Create `gwt-design` SKILL.md + references/ (DDD-embedded design skill absorbing brainstorm, register, clarify, deepen).
- [P] [x] **T-079**: Create `gwt-plan` SKILL.md + references/ (SDD-embedded planning skill absorbing spec-plan, spec-tasks, spec-analyze).
- [P] [x] **T-080**: Create `gwt-build` SKILL.md + references/ (TDD-embedded build skill absorbing spec-implement, standalone mode).
- [P] [x] **T-081**: Create `gwt-review` SKILL.md + references/ (new architecture feedback loop skill).

### 5b. Integration Skills (consolidated)

- [P] [x] **T-082**: Create `gwt-issue` SKILL.md + references/ (unified issue-register + issue-resolve with auto-detect).
- [P] [x] **T-083**: Rewrite `gwt-pr` SKILL.md + references/ (unified pr + pr-check + pr-fix with auto-detect).
- [P] [x] **T-084**: Create `gwt-search` SKILL.md (unified spec-search + issue-search + project-search).
- [P] [x] **T-085**: Create `gwt-agent` SKILL.md (unified agent-discover + agent-read + agent-send + agent-lifecycle).

### 5c. Cleanup

- [x] **T-086**: Move `spec_artifact.py` to `.claude/scripts/spec_artifact.py` (shared location).
- [x] **T-087**: Delete 21 old skill directories.
- [x] **T-088**: Update AGENTS.md with new 8-skill structure and recommended workflow.
- [x] **T-089**: Update SPEC-9 spec.md with US-5 through US-7, FR-024 through FR-035.
- [x] **T-090**: Update SPEC-9 tasks.md with Phase 5 tasks.

### 5d. Verification

- [ ] **T-091**: Verify `gwt-design` standalone invocation works.
- [ ] **T-092**: Verify `gwt-plan SPEC-X` standalone invocation works.
- [ ] **T-093**: Verify `gwt-build` standalone TDD mode works.
- [ ] **T-094**: Verify `gwt-review` generates report on gwt repository.
- [ ] **T-095**: Verify `gwt-issue`, `gwt-pr`, `gwt-search`, `gwt-agent` auto-detect modes.
- [ ] **T-096**: Verify design → plan → build → review chain suggestions.
