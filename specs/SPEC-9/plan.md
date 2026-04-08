# SPEC-9: Infrastructure -- Implementation Plan

## Phase 1: Docker UI Restoration

**Goal**: Restore Docker integration screens from the old TUI (v6.30.3) to the current ratatui-based TUI.

### Approach

Reference the old TUI implementation files (`docker_progress.rs`, `service_select.rs`, `port_select.rs`) for design patterns and state machine structure. Reimplement using current ratatui widget patterns and the existing screen navigation system.

### Key Changes

1. **gwt-tui**: Add `DockerProgress` screen with 5-state FSM (DetectingFiles, BuildingImage, StartingContainer, WaitingForServices, Ready).
   - Each state renders a progress indicator and status message.
   - Transitions driven by an app-side producer that bridges `gwt-docker` operations into progress messages.

2. **gwt-tui**: Add `ServiceSelect` screen.
   - Parse docker-compose.yml via gwt-core to list services.
   - Selectable list with service name and image.

3. **gwt-tui**: Add `PortSelect` screen.
   - Display conflicting ports with current and proposed mappings.
   - Allow user to edit port mappings inline.

4. **gwt-docker + gwt-tui**: Add a background producer for progress states.
   - File detection, image build, container start, and service readiness checks should emit `DockerProgressMessage` updates without inventing a fake `DockerManager`.

5. **gwt-tui**: Add container lifecycle controls (start/stop/restart) accessible from the Docker status area.

### Dependencies

- `gwt-docker` synchronous detection and lifecycle APIs.
- docker CLI available on the system.

## Phase 2: Embedded Skills — Build-Time Bundling

**Goal**: Bundle all skill, command, and hook files into the gwt binary at build time. Remove the legacy BuiltinSkill enum and unused SKILL_CATALOG.

### Key Changes

1. **gwt-core (build.rs)**: Replace the current SKILL_CATALOG generator with:
   - YAML frontmatter validation of all SKILL.md files using `serde_yaml` (build dependency only).
   - `cargo:rerun-if-changed` directives for `.claude/skills/`, `.claude/commands/`, `.claude/hooks/scripts/`.
   - Remove the `parse_frontmatter()` function and `SkillCatalogEntry` generation.

2. **gwt-skills**: Add `include_dir` crate to embed file directories:
   - `CLAUDE_SKILLS: Dir` — `.claude/skills/` (all subdirectories recursively)
   - `CLAUDE_COMMANDS: Dir` — `.claude/commands/` (all .md files)
   - `CLAUDE_HOOKS: Dir` — `.claude/hooks/scripts/` (all .mjs files)

3. **gwt-skills (registry.rs)**: Remove `BuiltinSkill` enum, `register_builtins()`, `to_embedded()`, `all()`. Remove all related tests.

4. **gwt-tui (settings.rs)**: Remove `skill_fields()` function. Replace Skills settings category content with a read-only display of bundled skill count (no toggle).

5. **gwt-tui (model.rs)**: Remove `embedded_skills: SkillRegistry` field and `register_builtins()` call from `Model::new()`.

### Dependencies

- `include_dir` crate (build dependency)
- `serde_yaml` crate (build dependency for validation only)

## Phase 2b: Embedded Skills — Runtime Distribution

**Goal**: Distribute bundled skills to target worktrees on every agent launch.

### Key Changes

1. **gwt-skills**: Add `distribute` module with:
   - `distribute_to_worktree(worktree_path: &Path) -> Result<DistributeReport>` — writes all bundled files to target.
   - Distribution targets: `.claude/skills/gwt-*/`, `.claude/commands/gwt-*.md`, `.claude/hooks/scripts/gwt-*.mjs`, `.codex/skills/gwt-*/`, `.codex/hooks/scripts/gwt-*.mjs`.
   - Full overwrite strategy: all gwt-managed files are replaced unconditionally.

2. **gwt-skills**: Add `git_exclude` module:
   - Reads/creates `.git/info/exclude`.
   - Manages gwt-managed block delimited by `# gwt-managed-begin` / `# gwt-managed-end`.
   - Adds exclude patterns for all distributed asset paths plus generated `.codex/hooks.json`.

3. **gwt-skills**: Add `settings_local` module:
   - Generates `.claude/settings.local.json` and untracked `.codex/hooks.json` from a shared typed hook builder.
   - Preserves user-defined hooks while replacing only gwt-managed runtime hooks.
   - Uses direct shell commands that write `GWT_SESSION_RUNTIME_PATH` instead of a Node-based runtime forwarder.
   - Skips tracked `.codex/hooks.json` files by default, but migrates tracked files that still contain gwt's legacy runtime forward hooks so launched worktrees do not stay pinned to stale Node-based runtime hooks.

4. **gwt-tui (app.rs)**: Call `distribute_to_worktree()` in agent launch flow, after `PaneManager::launch_agent()` resolves the worktree path.
   - Bootstrap a PID-scoped `Running` runtime sidecar immediately after successful PTY spawn, because interactive Codex may not emit `SessionStart` before the first prompt.

5. **gwt-agent (launch.rs)**:
   - Enable Codex hooks explicitly in every gwt-managed Codex launch (`--enable codex_hooks`).
   - Keep the flag alongside the existing web-search feature enablement so Codex hook execution does not depend on per-user `config.toml` state.

### Dependencies

- Phase 2 (bundled assets available at runtime)
- Bundled Claude/Codex hook assets

## Phase 2b.6: Unified Node-Based Managed Runtime Hook (US-9 / US-10)

**Goal**: Replace the platform-split `sh -lc` / `powershell` runtime-state commands with a single `node .../gwt-runtime-state.mjs <event>` invocation on both Claude and Codex targets, formalize the four existing PreToolUse Bash guard hooks (`gwt-block-*.mjs`) in the spec, and extend legacy detection so older sh/powershell gwt-managed entries are migrated alongside the existing `gwt-forward-hook.mjs` path.

### Motivation

- The four guard hooks (`gwt-block-cd-command.mjs`, `gwt-block-file-ops.mjs`, `gwt-block-git-branch-ops.mjs`, `gwt-block-git-dir-override.mjs`) are already wired via `bash_blockers_hook()` but have no spec coverage (no User Story, no FR, no SC). US-9 closes the documentation gap without touching runtime behavior.
- The runtime-state hook is currently split between `posix_runtime_hook_command()` (`sh -lc '...'`) and `powershell_runtime_hook_command()` (`powershell -NoProfile -Command "..."`). Maintaining two ~100-char inlined shell payloads has produced repeated quoting/portability bugs (tracked `.codex/hooks.json` written on the opposite host, shell-mismatch migration code, etc.).
- Earlier "no-Node runtime hook" guidance (FR-022 previous revision, SC-010 previous revision) was aimed at banning the `gwt-forward-hook.mjs` behavior of spawning a secondary `gwt`/`gwt-tauri` subprocess — a behavior that failed under interactive Codex sandboxing. A Node runtime hook that only writes the sidecar and exits does not exhibit the banned behavior; US-10 reintroduces Node for the write-side logic while preserving the "no secondary subprocess" invariant through FR-055 and the subprocess contract in the Regression Guardrail section.

### Approach

1. **Introduce `gwt-runtime-state.mjs`** as a new bundled script under `.claude/hooks/scripts/` and `.codex/hooks/scripts/`. CLI: `node gwt-runtime-state.mjs <event>`. Reads `GWT_SESSION_RUNTIME_PATH` from env, writes `{status, updated_at, last_activity_at, source_event}` atomically via temp-file + rename, exits 0 on all error paths. **Must not spawn any secondary process.** Contract specified in `contracts/runtime-state-hook-cli.md`.

2. **Replace the platform-split command generator** in `crates/gwt-skills/src/settings_local.rs`:
   - Remove `posix_runtime_hook_command()` and `powershell_runtime_hook_command()` and their `managed_hook_shell()` dispatcher.
   - Introduce `node_runtime_hook_command(event, script_root)` that returns `format!("node {}/gwt-runtime-state.mjs {}", script_root, event)`.
   - The `command` string is byte-identical on POSIX and Windows modulo `script_root`.
   - Add `env: { "GWT_MANAGED_HOOK": "runtime-state" }` to the hook entry where the hook schema supports it, and keep the marker embedded in the command string for backwards-compatible legacy detection.

3. **Extend legacy detection** in `tracked_codex_hooks_need_runtime_migration()`:
   - Keep the existing `contains_legacy_runtime_forwarder()` that matches `gwt-forward-hook.mjs`.
   - Replace `contains_managed_runtime_shell_mismatch()` with `contains_legacy_runtime_shell_command()` that fires whenever a managed entry with `GWT_MANAGED_HOOK=runtime-state` is still delivered via `sh -lc ` or `powershell -NoProfile -Command`, regardless of host. The new direction treats *every* shell-based runtime entry as legacy, so the mismatch helper's host check disappears.
   - Add `contains_node_runtime_hook()` helper that returns true when all gwt-managed entries point at `gwt-runtime-state.mjs`. Used by `tracked_codex_hooks_need_runtime_migration()` to short-circuit tracked files that already conform.

4. **Formalize guard hook generation** by keeping `bash_blockers_hook()` unchanged but:
   - Add a doc comment referencing FR-050 ~ FR-054.
   - Add a new unit test (`pretooluse_bash_blockers_match_spec_order`) that asserts the exact four-entry ordering required by FR-050.

5. **Distribution hygiene**:
   - Remove `gwt-forward-hook.mjs` from the active bundled asset list (`GWT_FORWARD_SCRIPT` constant becomes a legacy detection string only).
   - Ensure `distribute_to_worktree()` never writes `gwt-forward-hook.mjs` into an untracked target.
   - Keep the legacy detection constant (`const LEGACY_FORWARD_SCRIPT: &str = "gwt-forward-hook.mjs";`) so migration still fires on old tracked hooks files.

6. **Test migration**: Flip `assert!(!command.contains("node"))` assertions in `settings_local.rs` tests at lines 439, 570, and 750 to `assert!(command.contains("gwt-runtime-state.mjs"))` (and add a tighter `assert!(!command.contains(" sh -lc ") && !command.contains("powershell "))` check). Rewrite `posix_runtime_hook_command_writes_runtime_sidecar` into `node_runtime_hook_command_writes_runtime_sidecar` that spawns `node` on the bundled script in a temp dir and verifies the sidecar shape.

### Key Changes

1. **gwt-skills (new file)**: `crates/gwt-skills/src/runtime_hook.rs` — optional extraction of the node-based runtime hook command generator if `settings_local.rs` crosses the 500-line constitution threshold after the change. Keep inline initially; extract only if file grows past 500 lines.

2. **gwt-skills (modified)**: `crates/gwt-skills/src/settings_local.rs`
   - Remove: `posix_runtime_hook_command`, `powershell_runtime_hook_command`, `managed_hook_shell`, `command_shell_mismatch`, `contains_managed_runtime_shell_mismatch`.
   - Add: `node_runtime_hook_command(event, script_root) -> String`, `contains_legacy_runtime_shell_command`, `contains_node_runtime_hook`.
   - Update: `managed_hooks()` callers to use the node-based command; `tracked_codex_hooks_need_runtime_migration()` to OR all three legacy detectors.
   - Update: `bash_blockers_hook()` doc comment referencing FR-050.

3. **gwt-skills (new bundled asset)**: `.claude/hooks/scripts/gwt-runtime-state.mjs` and `.codex/hooks/scripts/gwt-runtime-state.mjs` (identical content). Write-only sidecar script; no subprocess spawn; exits 0 on any error.

4. **gwt-skills (removed bundled asset)**: `.claude/hooks/scripts/gwt-forward-hook.mjs` and `.codex/hooks/scripts/gwt-forward-hook.mjs` are removed from `CLAUDE_HOOKS` / `CODEX_HOOKS` include_dir bundling but kept in the source tree **only** if needed for cross-reference tests; otherwise deleted. The `gwt-forward-hook.mjs` string remains hard-coded inside legacy detection.

5. **Tests (modified)**: `crates/gwt-skills/src/settings_local.rs` inline tests T-047, T-048, T-131, T-132, T-147, T-812 (line refs may shift). Assertions flip from "no node" to "contains gwt-runtime-state.mjs".

6. **Tests (new)**:
   - `node_runtime_hook_command_is_byte_identical_across_platforms`
   - `pretooluse_bash_blockers_match_spec_order`
   - `contains_legacy_runtime_shell_command_matches_posix_sh`
   - `contains_legacy_runtime_shell_command_matches_windows_powershell`
   - `migration_replaces_posix_shell_runtime_with_node_form`
   - `migration_replaces_powershell_runtime_with_node_form`
   - `distribute_to_worktree_does_not_write_gwt_forward_hook`
   - `gwt_runtime_state_mjs_writes_sidecar_atomically` (integration: spawns `node` on the bundled script in a temp dir, verifies file shape, asserts no child process spawn via tracing)
   - `gwt_runtime_state_mjs_exits_zero_when_runtime_path_unset`
   - Unit tests for each guard hook (four tests covering FR-051 ~ FR-054) can be added as Node-based `node --test` cases under `.claude/hooks/scripts/__tests__/` if Node runtime is available in CI; otherwise gate them behind a feature flag and cover via the existing Rust integration smoke path.

### Dependencies

- Phase 2b (embedded hook distribution wired into agent launch) must remain in place.
- Node runtime available inside the worktree when agents run. Already required by existing guard hooks (`node .../gwt-block-*.mjs`), so no new runtime assumption.
- No new Cargo dependency.

### Constitution Impact

- **Rule 1 (Spec Before Implementation)**: Satisfied — spec.md US-9/US-10, FR-050~FR-058, SC-031~SC-037 added before code.
- **Rule 2 (Test-First Delivery)**: Satisfied — every FR maps to RED tasks before implementation tasks.
- **Rule 3 (No Workaround-First Changes)**: Satisfied — the flip of FR-022/SC-010 is rooted in the distinction between "subprocess spawning" (still banned) and "Node write-only script" (now the canonical path). Recorded in plan.md Motivation section and spec.md Regression Guardrail clarification.
- **Rule 4 (Minimal Complexity)**: The change removes two shell templates and a platform dispatcher and replaces them with a single command format. Net module complexity should decrease.
- **Rule 7 (File Size Rule)**: `settings_local.rs` is currently ~418 lines. The change nets roughly -80 lines (remove two templates, dispatcher, shell-mismatch helper) +40 lines (new Node helper, legacy shell detector), staying under 500.

### Risk Mitigation

- **Risk**: Agents launched with the new gwt version and a tracked `.codex/hooks.json` that already uses the unified Node form would accidentally be rewritten, dirtying tracked sources.
  **Mitigation**: `contains_node_runtime_hook()` short-circuit in `tracked_codex_hooks_need_runtime_migration()` — tracked files that already match the new shape are left alone. Covered by a new RED test.
- **Risk**: An old gwt version running in parallel against the same worktree regenerates `settings.local.json` back to `sh -lc` form.
  **Mitigation**: Out of scope; parallel version skew is a known limitation. Note it in plan.md but do not add version negotiation logic.
- **Risk**: `gwt-runtime-state.mjs` accidentally regresses to spawning `gwt`/`gwt-tauri` during a future refactor.
  **Mitigation**: Integration test traces child processes and fails if any are spawned. Contract documented in `contracts/runtime-state-hook-cli.md`.

## Phase 2c: Embedded Skills — Quality Improvement

**Goal**: Rewrite all 21 SKILL.md files to comply with Anthropic's skill authoring guidelines.

### Key Changes

1. **All SKILL.md description fields**: Rewrite in third-person voice with specific trigger phrases, under 250 characters for the front-loaded key use case.

2. **All SKILL.md frontmatter**: Add `allowed-tools`, `argument-hint`, and other applicable fields per skill.

3. **All SKILL.md body content**: Rewrite in imperative/infinitive form. Keep under 500 lines.

4. **Progressive Disclosure**: Extract detailed logic from SKILL.md into `references/` subdirectories for complex skills (gwt-pr-fix, gwt-spec-ops, gwt-spec-implement, etc.).

### Dependencies

- None (can run in parallel with Phase 2/2b)

## Phase 3: Historical hooks.rs Utility Completion

**Goal**: Preserve and finish the generic `hooks.rs` utility work carried over from archived SPEC-1786. This phase is historical support for the generic helper; the active Claude/Codex runtime-hook path no longer depends on it.

### Carried-Over Progress

The following capabilities from SPEC-1786 are already implemented:

- Generic `hooks.rs` safe-merge helpers with merge mode.
- User hook preservation during managed hook updates.
- gwt-managed hook identification via marker field.
- Confirmation dialog for Codex agents.
- Basic JSON corruption detection.

### Remaining Work

1. **Polish (Phase 3 from SPEC-1786)**:
   - Improve corruption recovery: timestamped backup creation and last-known-good restoration.
   - Edge case handling: concurrent writes, symlinked hooks.json, empty file.
   - Error message improvements for merge failures.

2. **Manual E2E (Phase 4 from SPEC-1786)**:
   - End-to-end verification of merge across multiple update cycles.
   - Corruption injection and recovery verification.
   - Concurrent gwt instance merge behavior.

### Dependencies

- Existing `hooks.rs` implementation in gwt-skills.

## Phase 4: Build Distribution

**Goal**: Finalize cross-platform build and distribution via GitHub Release and npm.

### Key Changes

1. **CI/CD**: Verify and fix the GitHub Actions release workflow.
   - Matrix build for 4 targets: macOS arm64, macOS x86_64, Linux x86_64, Windows x86_64.
   - Binary upload to GitHub Release.

2. **npm**: Verify postinstall script downloads correct platform binary.
   - Test on macOS arm64, macOS x86_64, Linux x86_64.
   - Handle offline/timeout gracefully.

3. **Version automation**: Verify Conventional Commits parsing, git-cliff CHANGELOG generation, and version bump flow.

### Dependencies

- GitHub Actions runners with cross-compilation toolchains.
- npm registry access.

## Phase 5: Skill Consolidation

**Goal**: Replace the fragmented skill surface with the methodology-based 8-skill system.

### Key Changes

1. Consolidate design / plan / build / review flows into standalone methodology skills.
2. Consolidate issue / PR / search / agent operations into auto-detect integration skills.
3. Update AGENTS and bundled skills so the repo, distributed assets, and workflow guidance stay aligned.

### Dependencies

- Embedded skill bundling and runtime distribution from Phase 2 / 2b.

## Phase 6: Search Runtime Contract Recovery

**Goal**: Keep unified search working across upgrades by restoring the shared runtime and documenting stable action names.

### Key Changes

1. Add repo-tracked project-index runtime assets and requirements under `gwt-core`.
2. Repair `~/.gwt/runtime/chroma_index_runner.py` and `~/.gwt/runtime/chroma-venv` during startup and workspace initialization.
3. Standardize file-search actions on `index-files` / `search-files` while preserving `index` / `search` aliases.
4. Update `gwt-search` family skills so `index-issues` examples include `--project-root`.
5. Validate bootstrap Python candidates before venv creation, keep working Store/launcher entrypoints, and translate only true no-candidate cases into install guidance.
6. Restore `gwt-project-search` as the canonical standalone semantic project-search skill / command while keeping `search-files` / `index-files` as the internal runner action names.
7. Split file indexing into code/docs collections so `search-files` stays implementation-focused while noisy skill/spec/snapshot artifacts are excluded from the code collection.

### Dependencies

- SPEC-10 workspace/runtime bootstrap flow.
- Shared Python availability for project-index setup.

## Risk Mitigation

- **Docker UI complexity**: Start with the progress screen FSM (simplest), then add service select and port select incrementally.
- **Hooks merge concurrency**: Use file locking (flock on Linux/macOS) to prevent concurrent write corruption.
- **Cross-compilation failures**: Maintain CI matrix with per-platform build verification; do not block release on Windows if other platforms succeed.
