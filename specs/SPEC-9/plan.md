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

5. **gwt-agent (launch.rs)**:
   - Enable Codex hooks explicitly in every gwt-managed Codex launch (`--enable codex_hooks`).
   - Keep the flag alongside the existing web-search feature enablement so Codex hook execution does not depend on per-user `config.toml` state.

### Dependencies

- Phase 2 (bundled assets available at runtime)
- Bundled Claude/Codex hook assets

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

## Risk Mitigation

- **Docker UI complexity**: Start with the progress screen FSM (simplest), then add service select and port select incrementally.
- **Hooks merge concurrency**: Use file locking (flock on Linux/macOS) to prevent concurrent write corruption.
- **Cross-compilation failures**: Maintain CI matrix with per-platform build verification; do not block release on Windows if other platforms succeed.
