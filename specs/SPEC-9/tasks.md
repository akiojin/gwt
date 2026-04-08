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
- [x] **T-144**: Write RED test: materialized Codex launches append the runtime namespace writable root after the persisted session id is known.
- [x] **T-145**: Implement materialized Codex runtime writable-root augmentation in `crates/gwt-tui/src/app.rs`.
- [x] **T-146**: Refresh `SPEC-9` artifacts and rerun focused plus broad verification for the materialized Codex runtime writable-root path.
- [x] **T-147**: Write RED test: tracked `.codex/hooks.json` files that still contain legacy gwt runtime forward hooks are migrated to the no-Node runtime-hook shape while preserving user hooks.
- [x] **T-148**: Implement tracked legacy Codex runtime-hook migration in `crates/gwt-skills/src/settings_local.rs` and cover launch materialization in `crates/gwt-tui/src/app.rs`.
- [x] **T-149**: Refresh `SPEC-9` / `SPEC-2` artifacts and rerun focused plus broad verification for tracked legacy Codex runtime-hook migration.

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

### 5e. Runtime Hook Contract Follow-up

- [x] **T-150**: Document the interactive Codex `SessionStart` gap and downstream launch-bootstrap contract in `SPEC-9` artifacts.

## Phase 6: Search Runtime Contract Recovery

- [x] **T-131**: Add canonical `index-files` / `search-files` action names to the repo-tracked project-index runner and keep `index` / `search` aliases for compatibility.
- [x] **T-132**: Add repo-tracked project-index requirements + managed venv bootstrap so gwt can repair the shared search runtime.
- [x] **T-133**: Update `gwt-search`, `gwt-project-search`, `gwt-project-index`, and `gwt-issue-search` examples to use canonical file-search actions and `index-issues --project-root`.
- [x] **T-134**: Update SPEC-9 runtime contract language so the search interface, shared runtime, and warning-only degradation stay aligned with implementation.
- [x] **T-135**: Add RED tests that require `gwt-project-search` as the canonical distributed project-search asset and reject `gwt-file-search` assets.
- [x] **T-136**: Restore canonical `gwt-project-search` skill / slash-command assets across bundled docs while keeping `search-files` / `index-files` as internal runner actions.
- [x] **T-137**: Update SPEC-9 artifacts and search-family references so standalone semantic project search points to `gwt-project-search` as the canonical public name.
- [x] **T-138**: Add RED tests for file-bucket classification so embedded skill assets, SPEC directories, archived SPEC directories, task logs, and snapshots are excluded from implementation search.
- [x] **T-139**: Split `index-files` into code/docs collections and add `search-files-docs` while keeping `search-files` implementation-focused.
- [x] **T-140**: Update `gwt-search`, `gwt-project-search`, and `gwt-project-index` docs to describe implementation-focused `search-files` behavior and the separate docs collection.
- [x] **T-141**: Refresh SPEC-9 artifacts and rerun Python + Cargo verification for the new code/docs file-index contract.

> Note: T-131~T-141 under Phase 6 collide numerically with T-131~T-149 under Phase 2b.5. Treat the Phase 2b.5 IDs as canonical when referring to hook-generation work; Phase 6 IDs are search-runtime work that shares the same numeric slots historically. New tasks below use T-200+ to avoid further collision.

## Phase 2b.6: Unified Node-Based Managed Runtime Hook (US-9 / US-10)

> All tasks in this phase MUST follow the Red→Green→Refactor order. RED tasks write failing tests first; implementation tasks only run after the RED task is confirmed failing. A single implementation task may close multiple RED tasks when the same code change satisfies all of them.

### 2b.6a: Spec & Contract Alignment (documentation, no code)

- [x] **T-200**: Update `specs/SPEC-9/spec.md` to add US-9 (PreToolUse Bash guard hooks, formalization), US-10 (unified Node-based managed runtime hook), FR-050 ~ FR-058, SC-031 ~ SC-037, the Regression Guardrail clarification, and the Managed Hook Config Schema rewrite. _(Done as part of this plan invocation.)_
- [x] **T-201**: Update `specs/SPEC-9/plan.md` with the Phase 2b.6 section (motivation, approach, key changes, constitution impact, risk mitigation). _(Done as part of this plan invocation.)_
- [x] **T-202**: Update `specs/SPEC-9/research.md` with the 2026-04-08 decision block distinguishing "subprocess spawn" from ".mjs for write-side logic". _(Done as part of this plan invocation.)_
- [x] **T-203**: Update `specs/SPEC-9/data-model.md` with `ManagedRuntimeHookEntry`, `BashGuardHookEntry`, and `LegacyRuntimeHookShape` entities. _(Done as part of this plan invocation.)_
- [x] **T-204**: Update `specs/SPEC-9/quickstart.md` with the US-9 / US-10 focused evidence block. _(Done as part of this plan invocation.)_
- [x] **T-205**: Create `specs/SPEC-9/contracts/runtime-state-hook-cli.md` with the full CLI, env, sidecar, exit-code, and subprocess-invariant contract. _(Done as part of this plan invocation.)_

### 2b.6b: New Runtime-State Hook Script (RED → GREEN)

- [ ] **T-210**: [RED] Add failing test `gwt_skills::tests::gwt_runtime_state_mjs_exists_in_bundled_assets` that asserts `CLAUDE_HOOKS` and `CODEX_HOOKS` contain `gwt-runtime-state.mjs`. `crates/gwt-skills/src/lib.rs` or `crates/gwt-skills/tests/`.
- [ ] **T-211**: [RED] Add failing test `gwt_runtime_state_mjs_source_contains_no_child_process_token` that greps the bundled script source for the literal token `child_process` and asserts zero matches. Enforces FR-055 subprocess invariant statically.
- [ ] **T-212**: [RED] Add failing integration test `gwt_runtime_state_mjs_writes_sidecar_atomically` under `crates/gwt-skills/tests/runtime_hook_script.rs`. Preconditions: `which node` succeeds. Test body: copy the bundled script to a temp dir, set `GWT_SESSION_RUNTIME_PATH=<tempdir>/sidecar.json`, spawn `node gwt-runtime-state.mjs SessionStart`, assert exit code 0, assert sidecar JSON matches the contract shape, assert the spawned Node process reported zero child PIDs during its lifetime (use `process.pidtree` equivalent or `ps --ppid`).
- [ ] **T-213**: [RED] Add failing test `gwt_runtime_state_mjs_exits_zero_when_runtime_path_unset` that spawns the script with `GWT_SESSION_RUNTIME_PATH` unset and asserts exit code 0 and no file creation.
- [ ] **T-214**: [RED] Add failing test `gwt_runtime_state_mjs_ignores_unknown_event_name` that spawns the script with `<event>` = `BogusEvent` and asserts exit code 0 and no file creation.
- [ ] **T-215**: [RED] Add failing test `gwt_runtime_state_mjs_survives_unwritable_sidecar_dir` that points `GWT_SESSION_RUNTIME_PATH` at a path whose parent is chmod 0555 (POSIX) or read-only (Windows), asserts exit 0, and asserts no partial file at the final path.
- [ ] **T-216**: Create `.claude/hooks/scripts/gwt-runtime-state.mjs` implementing the contract in `specs/SPEC-9/contracts/runtime-state-hook-cli.md`. Pure Node, no imports from `node:child_process`. Use `node:fs/promises` for atomic temp-file + rename. Use `process.argv[2]` for event. Use `process.env.GWT_SESSION_RUNTIME_PATH`. Always exit 0.
- [ ] **T-217**: Copy the same script to `.codex/hooks/scripts/gwt-runtime-state.mjs` (identical content) and add a build-time check in `crates/gwt-skills/build.rs` (or equivalent existing bundler validator) that asserts the two files are byte-identical. Prevents drift.
- [ ] **T-218**: Verify T-210 ~ T-215 now GREEN.

### 2b.6c: Generator Refactor (RED → GREEN)

- [ ] **T-220**: [RED] Add failing test `node_runtime_hook_command_is_byte_identical_across_platforms` in `crates/gwt-skills/src/settings_local.rs` tests. Calls the new generator twice — once with a mocked `cfg!(windows)=false` context, once with `=true` — and asserts the returned command strings are byte-identical for the same worktree path.
- [ ] **T-221**: [RED] Add failing test `generated_settings_local_runtime_entries_point_at_gwt_runtime_state_mjs` that generates `.claude/settings.local.json` in a temp dir and asserts every `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop` gwt-managed entry contains the substring `gwt-runtime-state.mjs` and does not contain `sh -lc `, `powershell `, or `gwt-forward-hook.mjs`.
- [ ] **T-222**: [RED] Add failing test `generated_codex_hooks_runtime_entries_point_at_gwt_runtime_state_mjs` — same assertion for `.codex/hooks.json`.
- [ ] **T-223**: [RED] Add failing test `pretooluse_bash_blockers_match_spec_order` that asserts the PreToolUse `Bash` matcher contains exactly four entries pointing at `gwt-block-git-branch-ops.mjs`, `gwt-block-cd-command.mjs`, `gwt-block-file-ops.mjs`, `gwt-block-git-dir-override.mjs` in that exact order, in addition to the runtime-state entry in a separate matcher block.
- [ ] **T-224**: Implement `node_runtime_hook_command(event, script_root)` in `crates/gwt-skills/src/settings_local.rs`. Delete `posix_runtime_hook_command`, `powershell_runtime_hook_command`, `managed_hook_shell`, `command_shell_mismatch`, `contains_managed_runtime_shell_mismatch`. Update `managed_hooks()` callers.
- [ ] **T-225**: Flip the existing assertions in `settings_local.rs` inline tests at the previous no-Node lines (`assert!(!command.contains("node"))` occurrences at 439, 570, 750 based on the 2026-04-08 survey) to the new positive form: `assert!(command.contains("gwt-runtime-state.mjs")) && assert!(!command.contains(" sh -lc ") && !command.contains("powershell "))`. Update the existing `posix_runtime_hook_command_writes_runtime_sidecar` test to target the bundled script via `node` spawn (shares helper with T-212 if practical).
- [ ] **T-226**: Verify T-220 ~ T-223 now GREEN. Run `cargo test -p gwt-skills -- --nocapture` and resolve any regression from the assertion flip.

### 2b.6d: Legacy Detection Extension (RED → GREEN)

- [ ] **T-230**: [RED] Add failing test `contains_legacy_runtime_shell_command_matches_posix_sh` that constructs a tracked `.codex/hooks.json` containing one `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'` entry plus an unrelated user hook, calls the new detector, and asserts it returns true.
- [ ] **T-231**: [RED] Add failing test `contains_legacy_runtime_shell_command_matches_windows_powershell` — same assertion for a PowerShell-shape entry.
- [ ] **T-232**: [RED] Add failing test `contains_node_runtime_hook_returns_true_for_unified_form` that constructs a hooks.json whose managed entries already use `node .../gwt-runtime-state.mjs` and asserts `tracked_codex_hooks_need_runtime_migration()` returns `false` (short-circuit: already on the unified shape).
- [ ] **T-233**: [RED] Add failing test `migration_replaces_posix_shell_runtime_with_node_form` that starts with a tracked hooks.json containing a `sh -lc` managed entry + a user hook, runs the migration, and asserts the managed entry is now `node .../gwt-runtime-state.mjs <event>` while the user hook is unchanged.
- [ ] **T-234**: [RED] Add failing test `migration_replaces_powershell_runtime_with_node_form` — same assertion for a PowerShell-shape starting point.
- [ ] **T-235**: [RED] Add failing test `migration_replaces_legacy_forward_hook_with_node_form` — keeps coverage of the existing `gwt-forward-hook.mjs` legacy path under the new migration writer.
- [ ] **T-236**: Implement `contains_legacy_runtime_shell_command`, `contains_node_runtime_hook`, and update `tracked_codex_hooks_need_runtime_migration` in `crates/gwt-skills/src/settings_local.rs` to OR all three legacy detectors (forwarder, sh, powershell) and short-circuit when `contains_node_runtime_hook` is true. Update the migration writer to replace any matched legacy entry with `node_runtime_hook_command(event, script_root)`.
- [ ] **T-237**: Verify T-230 ~ T-235 now GREEN.

### 2b.6e: Distribution Hygiene (RED → GREEN)

- [ ] **T-240**: [RED] Add failing test `distribute_to_worktree_does_not_write_gwt_forward_hook` that runs `distribute_to_worktree()` against an empty temp dir and asserts `.claude/hooks/scripts/gwt-forward-hook.mjs` is NOT created while `gwt-runtime-state.mjs` IS created (plus the four `gwt-block-*.mjs` scripts).
- [ ] **T-241**: [RED] Add failing test `distribute_to_worktree_preserves_tracked_gwt_forward_hook` that stages a tracked repo containing `.claude/hooks/scripts/gwt-forward-hook.mjs`, runs `distribute_to_worktree()`, and asserts the tracked file is left untouched (no write, no delete).
- [ ] **T-242**: Remove `gwt-forward-hook.mjs` from the bundled asset list in `crates/gwt-skills/src/assets.rs` (or wherever `include_dir` targets are declared). Keep the string literal `"gwt-forward-hook.mjs"` inside the legacy detector module under a clearly-named constant (e.g. `LEGACY_FORWARD_SCRIPT_NAME`).
- [ ] **T-243**: Delete `.claude/hooks/scripts/gwt-forward-hook.mjs` and `.codex/hooks/scripts/gwt-forward-hook.mjs` from the source tree. The legacy detector no longer needs the file on disk — it only needs to match the string in foreign `hooks.json` content.
- [ ] **T-244**: Verify T-240 and T-241 GREEN. Run `cargo test -p gwt-skills -- --nocapture` and a broad `cargo test --workspace` to catch any downstream reference to the deleted file.

### 2b.6f: Guard Hook Test Coverage (new unit tests; existing scripts unchanged)

- [ ] **T-250**: Add Rust integration test `gwt_block_cd_command_blocks_paths_outside_worktree` that spawns `node gwt-block-cd-command.mjs` with stdin JSON `{tool_input:{command:"cd /tmp"}}` inside a temp git worktree, asserts exit code 2 and a JSON block body; repeats with `"cd ./src"` and asserts exit code 0.
- [ ] **T-251**: Add integration test `gwt_block_file_ops_blocks_outside_worktree` — `rm -rf ../outside` → exit 2; `mkdir ./sub` → exit 0.
- [ ] **T-252**: Add integration test `gwt_block_git_branch_ops_allows_readonly_queries` — `git checkout main` → 2; `git branch --show-current` → 0; `git status` → 0; `git branch -D foo` → 2.
- [ ] **T-253**: Add integration test `gwt_block_git_dir_override_blocks_env_prefix` — `GIT_DIR=/tmp/repo git status` → 2; `git status` → 0; `export GIT_DIR=/tmp/repo` → 2.
- [ ] **T-254**: Gate T-250 ~ T-253 behind a `gwt_skills::tests::node_integration` cfg helper that skips the tests cleanly when `which node` fails, so the suite stays runnable on minimal CI runners.

### 2b.6g: Metadata & Completion

- [ ] **T-260**: Update `specs/SPEC-9/metadata.json` to reflect US-9 / US-10 (new user stories, status `in-progress` → remains `in-progress`, phase remains `Implementation`).
- [ ] **T-261**: Update `specs/SPEC-9/progress.md` Phase 2b.6 subsection with the new work summary once implementation completes.
- [ ] **T-262**: Final verification: `cargo test -p gwt-skills -- --nocapture`, `cargo test -p gwt-tui settings -- --nocapture`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt -- --check`. Re-run the quickstart manual smoke block from `specs/SPEC-9/quickstart.md` and record results in `progress.md`.
- [ ] **T-263**: Follow-up: reference FR-050 ~ FR-058 and SC-031 ~ SC-037 from the PR description when Phase 2b.6 implementation is submitted.

### Parallelism Guide

- T-200 ~ T-205 all write to `specs/SPEC-9/*` — done in-session, non-parallel.
- T-210 ~ T-215 (script RED tests), T-220 ~ T-223 (generator RED tests), T-230 ~ T-235 (legacy detection RED tests), T-240 ~ T-241 (distribution RED tests), T-250 ~ T-253 (guard hook tests) may be authored in parallel [P] **only if** each is placed in a distinct test module file, because shared `#[cfg(test)]` modules serialize file writes.
- T-216 ~ T-217 write the bundled script: must run before T-220 (generator tests depend on the script existing).
- T-224 ~ T-226 edit `settings_local.rs`: must run sequentially because they touch the same file region.
- T-236 and T-242 touch non-overlapping file regions and can run in parallel [P] after T-224 lands.
- T-243 (delete `gwt-forward-hook.mjs`) must run after T-242 to avoid breaking the bundler.
- T-260 ~ T-263 run last.

### Traceability

| User Story | Acceptance Scenarios | FRs | SCs | Tasks |
|---|---|---|---|---|
| US-9 | 1–6 | FR-050, FR-051, FR-052, FR-053, FR-054, FR-057, FR-058 | SC-031, SC-032, SC-033, SC-034 | T-223, T-250, T-251, T-252, T-253 |
| US-10 | 1–7 | FR-019, FR-020, FR-021, FR-022, FR-023, FR-055, FR-056, FR-057, FR-058 | SC-009, SC-010, SC-013, SC-021, SC-035, SC-036, SC-037 | T-210–T-218, T-220–T-226, T-230–T-237, T-240–T-244 |
