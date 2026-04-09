# Infrastructure -- Build Distribution, Docker UI, Embedded Skills, Hooks Merge

## Background

gwt infrastructure covers four domains: build/distribution (GitHub Release + bunx/npx), Docker integration UI (detection, container lifecycle, port mapping), embedded skill management, and managed hook configuration for Claude Code / Codex. Docker UI screens existed in the old TUI (v6.30.3) and need restoration to the current ratatui-based TUI. The older archived hooks.json merge work from SPEC-1786 remains as a generic utility in `hooks.rs`, but the active Claude/Codex runtime-hook path is now a typed config generator that writes `.claude/settings.local.json` and `.codex/hooks.json`, preserves user hooks, preserves tracked Codex hook files by default, and migrates tracked files that still contain any historical gwt-managed runtime-hook shape (legacy `gwt-forward-hook.mjs` forwarders, direct `sh -lc` runtime commands, or PowerShell runtime commands) to the unified Node-based runtime-hook shape that invokes `node .../gwt-runtime-state.mjs` to write `GWT_SESSION_RUNTIME_PATH`. Bundled PreToolUse Bash guard hooks (`gwt-block-cd-command.mjs`, `gwt-block-file-ops.mjs`, `gwt-block-git-branch-ops.mjs`, `gwt-block-git-dir-override.mjs`) reject worktree-escape patterns before the agent executes them. Embedded skill management also owns keeping the bundled `.claude/skills/gwt-*` assets aligned with the current local SPEC artifact model, including persisted `analysis.md`, and now covers the pre-SPEC intake entrypoint that interviews rough requests before any `spec.md` is drafted.

## User Stories

### US-1: Distribute gwt via GitHub Release and bunx/npx (P0) -- PARTIALLY IMPLEMENTED

As a user, I want to install gwt via `bunx gwt` or `npx gwt` or download a binary from GitHub Releases so that I can get started quickly on any platform.

**Acceptance Scenarios**

1. Given a new release is tagged, when the release workflow runs, then binaries for macOS arm64, macOS x86_64, Linux x86_64, and Windows x86_64 are uploaded to GitHub Releases.
2. Given I run `bunx gwt` or `npx gwt`, when the postinstall script runs, then the correct binary for my platform is downloaded and executed.
3. Given Conventional Commits are used, when a release is created, then the version number and CHANGELOG are generated automatically.

### US-2: Detect Docker/DevContainer and Launch Agents in Containers (P1) -- NOT IMPLEMENTED

As a developer, I want gwt to detect Docker environments and launch agents inside containers so that I can develop in containerized environments seamlessly.

**Acceptance Scenarios**

1. Given a project has a Dockerfile or docker-compose.yml, when gwt starts, then Docker is detected and the Docker workflow is offered.
2. Given Docker is detected, when I select the Docker workflow, then a progress screen shows: DetectingFiles, BuildingImage, StartingContainer, WaitingForServices, Ready.
3. Given a docker-compose.yml with multiple services, when building, then I can select which service to use.
4. Given a port conflict exists, when starting a container, then a port selection screen allows me to resolve the conflict.
5. Given a running container, when I use the container management UI, then I can start, stop, or restart the container.
6. Given a .devcontainer/devcontainer.json exists, when gwt starts, then DevContainer detection is offered as an alternative.

### US-3: Distribute Embedded Skills to Worktrees on Agent Launch (P1) -- IMPLEMENTED

As a developer, I want gwt to bundle all embedded skills, commands, and hooks into the binary and distribute them to the target worktree every time an agent is launched, so that agents always have up-to-date skill definitions without manual configuration.

**Acceptance Scenarios**

1. Given an agent is launched from gwt, when the launch completes, then `.claude/skills/`, `.claude/commands/`, `.claude/hooks/`, `.codex/skills/`, and `.codex/hooks/scripts/` are written to the target worktree with the bundled skill files.
2. Given the target worktree already has older untracked gwt-managed skill files, when an agent is launched, then those generated files are overwritten with the latest bundled versions.
3. Given the target worktree tracks `.claude/*` or `.codex/*` gwt asset paths in Git, when an agent is launched, then distribution preserves those tracked files and only writes untracked generated targets.
4. Given the target worktree contains stale `gwt-*` skill, command, or hook paths that are not part of the current embedded bundle, when an agent is launched, then those stale paths are deleted from the managed asset trees before the current bundle is materialized, regardless of whether the stale paths are tracked by Git.
5. Given gwt starts and loads the current repo plus active worktree inventory, when stale `gwt-*` paths remain from older bundled surfaces, then those stale paths are pruned from the repo root and active worktrees without materializing missing bundle assets solely because the TUI refreshed its metadata.
6. Given an agent is launched, when skill distribution completes, then `.claude/settings.local.json` and `.codex/hooks.json` for untracked worktrees or tracked worktrees that still carry any historical gwt-managed runtime-hook shape are generated or migrated to the unified Node-based runtime-hook form, preserving any existing user-defined hooks while replacing only gwt-managed runtime entries.
7. Given an agent is launched, when skill distribution completes, then `.git/info/exclude` in the worktree is updated to exclude gwt-managed asset paths (`.claude/skills/gwt-*`, `.claude/commands/gwt-*`, `.claude/hooks/scripts/gwt-*`, `.codex/skills/gwt-*`, `.codex/hooks/scripts/gwt-*`, `.claude/settings.local.json`, `.codex/hooks.json`). Distributed hook scripts always include `gwt-runtime-state.mjs`, `gwt-block-cd-command.mjs`, `gwt-block-file-ops.mjs`, `gwt-block-git-branch-ops.mjs`, and `gwt-block-git-dir-override.mjs`.
8. Given the gwt binary is built, when build.rs runs, then all SKILL.md files are validated for YAML frontmatter syntax errors, and the build fails with a clear error if any SKILL.md has malformed YAML.
9. Given all skills are bundled, when the binary starts, then no runtime file I/O is needed to read skill definitions — skills are embedded in the binary via `include_dir`.

### US-4: Generate Managed Claude/Codex Hook Configs Preserving User Hooks (P1) -- IMPLEMENTED

As a developer, I want gwt to generate managed Claude/Codex hook configs without overwriting my custom hooks so that both gwt automation and my personal hooks coexist.

**Acceptance Scenarios**

1. Given `.claude/settings.local.json` or an untracked `.codex/hooks.json` contains user-defined hooks, when gwt updates its managed runtime hooks, then user hooks are preserved.
2. Given a prior config contains stale gwt-managed runtime hooks, when gwt regenerates the file, then only the gwt-managed runtime entries are replaced.
3. Given Codex runtime hooks are generated for an untracked worktree, when the file is written, then every live-state hook entry is a `node` invocation of the bundled `gwt-runtime-state.mjs` script that updates `GWT_SESSION_RUNTIME_PATH`, and no gwt-managed entry shells out through `sh -lc` or `powershell` directly.
4. Given `.codex/hooks.json` is tracked by Git in the target worktree and already uses the unified Node-based runtime-hook form, when an agent launches, then gwt does not rewrite that file and does not dirty tracked source files.
5. Given `.codex/hooks.json` is tracked by Git in the target worktree and still contains any historical gwt-managed runtime-hook shape (legacy `gwt-forward-hook.mjs` forwarder, direct `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'` command, or equivalent PowerShell command), when an agent launches, then gwt migrates only the gwt-managed runtime entries to the unified Node-based runtime-hook form while preserving user hooks.
6. Given gwt launches a Codex agent session, when the launch command is built, then Codex starts with the `codex_hooks` feature enabled so `hooks.json` actually executes.
7. Given interactive Codex does not emit `SessionStart` before the first prompt, when gwt launches that session successfully, then downstream launch code may bootstrap a `Running` runtime sidecar until the first real hook event overwrites it.
8. Given a gwt-managed runtime hook entry is invoked, when the Node script runs, then it reads `GWT_SESSION_RUNTIME_PATH` from the environment, writes a single runtime-state JSON file atomically, exits 0, and does not spawn any additional `gwt`/`gwt-tauri` subprocess.

## Edge Cases

- Docker daemon not running when Docker workflow is selected.
- docker-compose.yml references images that do not exist locally.
- Port conflict on a privileged port (below 1024).
- `.claude/settings.local.json` or `.codex/hooks.json` contains invalid JSON and must be treated as a recoverable empty-object input.
- `.codex/hooks.json` is tracked by Git in the target worktree and already uses the unified Node-based runtime-hook shape; it must not be rewritten.
- `.codex/hooks.json` is tracked by Git in the target worktree and still contains legacy `gwt-forward-hook.mjs` forwarders; those gwt-managed runtime entries must be migrated to the Node-based runtime-hook shape without dropping user hooks.
- `.codex/hooks.json` is tracked by Git in the target worktree and still contains direct `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'` or `powershell ... GWT_MANAGED_HOOK ... runtime-state ...` runtime commands from an older gwt version; those gwt-managed runtime entries must also be migrated to the unified Node-based runtime-hook shape.
- `gwt-runtime-state.mjs` is invoked with `GWT_SESSION_RUNTIME_PATH` unset, or the sidecar directory is not writable; the script must still exit 0 without blocking the agent.
- A Bash command passed to the agent mixes a safe prefix with an unsafe suffix separated by `;`, `&&`, `||`, or `|`; each guard hook must evaluate every segment, not just the first token.
- An agent attempts to bypass guard hooks by resolving a path via a symlink pointing outside the worktree; the guard hooks must compare resolved real paths against the worktree root.
- Codex has `hooks.json` available but the `codex_hooks` feature flag is not enabled at launch.
- Interactive Codex launches may not emit `SessionStart` before the first prompt, even though `hooks.json` is present and `codex_hooks` is enabled.
- Multiple gwt instances are running simultaneously; runtime hook commands must use the injected `GWT_SESSION_RUNTIME_PATH` instead of recomputing shared global paths.
- Target worktree is read-only or has insufficient disk space for skill distribution.
- `.git/info/exclude` does not exist (must be created).
- `.claude/settings.local.json` contains user-defined hooks that conflict with gwt-managed hooks.
- SKILL.md frontmatter contains YAML syntax errors (caught at build time).
- Agent launch is interrupted mid-distribution (partial write).
- Target worktree tracks bundled `.claude/*` or `.codex/*` assets in Git; distribution must not dirty tracked source files.
- npm postinstall script runs in an environment without internet access.
- GitHub Release workflow runs but binary compilation fails on one platform.

## Regression Guardrail: Claude/Codex Runtime Hooks

Runtime-hook regressions repeatedly occurred when only one layer (config generation, launch args, or UI rendering) was validated in isolation. Hook reliability in this domain is defined by end-to-end sidecar observability, not by config file presence alone.

> **Clarification (SPEC-9 / US-10):** The earlier "no-Node runtime hook" rule was about the forbidden behavior of spawning a **secondary `gwt`/`gwt-tauri` subprocess** from inside a hook, which depended on PATH and app bundle layout and broke under interactive Codex sandboxing. Invoking `node <worktree>/.claude/hooks/scripts/gwt-runtime-state.mjs` to write the sidecar is **not** the same thing and is explicitly allowed by US-10: the Node script never spawns a downstream gwt subprocess, so the recurring failure pattern below still does not apply. Any Node-based hook that spawns `gwt`/`gwt-tauri` (or any secondary binary) is still a regression.

### Recurring failure pattern to preserve

1. `hooks.json` existed but Codex runtime hooks were inactive because launch omitted `--enable codex_hooks`.
2. `GWT_SESSION_RUNTIME_PATH` pointed outside the worktree, but Codex sandbox writable roots did not include `~/.gwt/sessions/runtime/<gwt-pid>`.
3. Tracked `.codex/hooks.json` files kept historical gwt-managed runtime-hook shapes (legacy `gwt-forward-hook.mjs` subprocess forwarders, direct `sh -lc` commands, or PowerShell commands) and did not receive unified Node runtime-hook migration.
4. Interactive Codex startup could delay `SessionStart`, so hook-only initialization left no early runtime sidecar.
5. Hook asset/settings distribution happened too late for first-turn hook events.
6. A runtime-state hook spawned `gwt` / `gwt-tauri` / any other binary as a secondary process, causing sandbox denials or PATH resolution failures.

### Mandatory cross-layer checks for this SPEC scope

- Launch contract: verify `--enable codex_hooks` and runtime writable-root injection on final materialized launch config.
- Config contract: verify effective worktree hook files (`.claude/settings.local.json`, `.codex/hooks.json`) are on the unified Node runtime-hook shape and contain no `sh -lc`, `powershell`, or `gwt-forward-hook.mjs` gwt-managed entries.
- Migration contract: verify tracked `.codex/hooks.json` files that carry any historical gwt-managed runtime-hook shape are migrated to the unified Node-based form while user hooks stay intact.
- Subprocess contract: verify `gwt-runtime-state.mjs` never spawns a secondary process; the only IO it performs is the sidecar write at `$GWT_SESSION_RUNTIME_PATH`.
- Runtime contract: verify PID-scoped sidecars are written/updated at `~/.gwt/sessions/runtime/<gwt-pid>/<session-id>.json`.
- Startup contract: verify interactive Codex sessions are visible before first prompt via launch bootstrap, then overwritten by real hook events.
- Guard contract: verify PreToolUse `Bash` matcher always includes `gwt-block-git-branch-ops.mjs`, `gwt-block-cd-command.mjs`, `gwt-block-file-ops.mjs`, and `gwt-block-git-dir-override.mjs` in addition to the managed runtime-state hook.

## Functional Requirements

### Build and Distribution

- **FR-001**: GitHub Release workflow produces cross-platform binaries: macOS arm64, macOS x86_64, Linux x86_64, Windows x86_64.
- **FR-002**: npm package with postinstall script that downloads the correct platform binary from GitHub Releases.
- **FR-003**: Conventional Commits parsing for automatic version detection (feat=minor, fix=patch, !=major), CHANGELOG generation via git-cliff, and Release automation.

### Docker UI (restore from old TUI)

- **FR-004**: Docker Progress screen with 5 states: DetectingFiles, BuildingImage, StartingContainer, WaitingForServices, Ready.
- **FR-005**: Service Select screen: list Docker Compose services from docker-compose.yml and allow selection.
- **FR-006**: Port Select screen: detect port conflicts and allow user to remap conflicting ports.
- **FR-007**: Container lifecycle management: start, stop, restart containers from TUI with status feedback.
- **FR-008**: DevContainer detection: parse .devcontainer/devcontainer.json and offer DevContainer launch workflow.

### Embedded Skills — Build-Time Bundling

- **FR-009**: All skill, command, and hook files under `.claude/skills/`, `.claude/commands/`, `.claude/hooks/scripts/`, and `.codex/hooks/scripts/` are embedded into the gwt binary at build time using `include_dir` crate. Embedded skill categories:
  - PR management: gwt-pr, gwt-pr-check, gwt-pr-fix
  - SPEC workflow: gwt-spec-brainstorm, gwt-spec-ops, gwt-spec-register, gwt-spec-implement, gwt-spec-clarify, gwt-spec-deepen, gwt-spec-plan, gwt-spec-tasks, gwt-spec-analyze, gwt-spec-search
  - Issue management: gwt-issue-register, gwt-issue-resolve, gwt-issue-search
  - Agent pane management: gwt-agent
  - Utilities: gwt-project-search, gwt-project-index, gwt-spec-to-issue-migration
  - PreToolUse Bash guards (Node-based `.mjs`): `gwt-block-cd-command.mjs`, `gwt-block-file-ops.mjs`, `gwt-block-git-branch-ops.mjs`, `gwt-block-git-dir-override.mjs`
  - Runtime-state writer (Node-based `.mjs`): `gwt-runtime-state.mjs`
  - Deprecated (retained only for legacy detection / migration, not registered): `gwt-forward-hook.mjs`
- **FR-010**: `build.rs` validates YAML frontmatter of every `SKILL.md` at compile time using `serde_yaml`. Malformed YAML causes a build failure with file path and error details.
- **FR-011**: The `BuiltinSkill` enum, `SKILL_CATALOG` constant, `register_builtins()` function, and `skill_fields()` in the TUI Settings screen are removed. Skill interpretation is the responsibility of Claude Code / Codex, not gwt.

### Embedded Skills — Runtime Distribution

- **FR-012**: On every agent launch, gwt writes bundled skill files to the target worktree. Distribution targets:
  - `.claude/skills/gwt-*/` — Claude Code skill definitions
  - `.claude/commands/gwt-*.md` — Claude Code slash commands
  - `.claude/hooks/scripts/gwt-*.mjs` — Claude Code hooks
  - `.codex/skills/gwt-*/` — Codex skill definitions (same content as Claude)
  - `.codex/hooks/scripts/gwt-*.mjs` — Codex hook scripts
- **FR-013**: Distribution overwrites untracked gwt-managed generated files on each agent launch.
- **FR-013a**: Distribution must skip writes for gwt-managed asset paths that are already tracked by Git in the target worktree, except `.claude/settings.local.json` (always regenerated) and tracked `.codex/hooks.json` files that require runtime-hook migration.
- **FR-013b**: Distribution removes stale `gwt-*` skill, command, and hook paths under `.claude/skills/`, `.claude/commands/`, `.claude/hooks/scripts/`, `.codex/skills/`, and `.codex/hooks/scripts/` by synchronizing each managed asset tree against the current embedded bundle. This includes stale root entries and stale nested paths inside retained managed directories, even if the stale paths are tracked by Git in the target worktree.
- **FR-013c**: During initial repo/worktree metadata load, gwt performs a prune-only sweep across the current repo root and active worktree paths so stale `gwt-*` managed paths from older bundled surfaces are removed even before the next agent relaunch. This sweep must not materialize missing bundle assets in worktrees that are merely being discovered.
- **FR-014**: `.claude/settings.local.json` is generated on each agent launch from a typed hook-config builder even when tracked, preserving non-gwt hooks and unrelated Claude settings while replacing only gwt-managed runtime hooks.
- **FR-014a**: `.codex/hooks.json` is generated on each agent launch when the file is untracked in the target worktree. Existing user hooks are preserved, gwt-managed runtime hooks are replaced, and tracked `.codex/hooks.json` files are left untouched unless they still contain legacy gwt-managed runtime forward hooks or gwt-managed runtime commands for a non-host shell.
- **FR-015**: `.git/info/exclude` is updated on each agent launch to exclude gwt-managed asset paths, including `.codex/hooks.json`. Existing user entries are preserved; gwt-managed entries are delimited by `# gwt-managed-begin` / `# gwt-managed-end` markers.

### Embedded Skills — Quality Standards (Anthropic Guidelines)

- **FR-016**: All SKILL.md `description` fields follow Anthropic guidelines: third-person voice, specific trigger phrases, front-loaded key use case within 250 characters.
- **FR-017**: All SKILL.md body content uses imperative/infinitive form, stays under 500 lines, and delegates detailed logic to `references/` subdirectories (Progressive Disclosure).
- **FR-018**: All SKILL.md frontmatter actively uses `allowed-tools`, `argument-hint`, and other applicable fields as defined by the Claude Code skill specification.

### Managed Runtime Hook Generation

- **FR-019**: Claude and Codex runtime hook configs are generated from a shared typed builder so both surfaces emit the same live-state event mapping and hook ordering.
- **FR-020**: Preserve user-defined hooks during gwt-managed runtime hook updates; only gwt-managed runtime entries are replaced.
- **FR-021**: gwt-managed runtime hooks are identified by a command marker (`GWT_MANAGED_HOOK=runtime-state`) and legacy command patterns (`gwt-forward-hook.mjs`, direct `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'`, and `powershell ... GWT_MANAGED_HOOK ... runtime-state ...`) during config sanitization.
- **FR-022**: Live-state runtime hooks are a single `node <worktree>/.claude/hooks/scripts/gwt-runtime-state.mjs <event>` invocation (and the `.codex/hooks/scripts/...` equivalent). The Node script writes `GWT_SESSION_RUNTIME_PATH` atomically and must not spawn any secondary process (no `gwt`, `gwt-tauri`, or any other binary). There is no platform-conditional shell fallback; the generated command is byte-identical on POSIX and Windows modulo the worktree path.
- **FR-023**: If `.codex/hooks.json` is tracked by Git in the target worktree, gwt preserves the tracked file unchanged unless it still contains any historical gwt-managed runtime-hook shape (legacy `gwt-forward-hook.mjs` forwarder, direct `sh -lc` runtime command, or PowerShell runtime command); in those cases, gwt migrates only the gwt-managed runtime entries to the unified Node-based runtime-hook form while preserving user hooks.
- **FR-023a**: Codex launch configs generated by gwt enable the `codex_hooks` feature flag so repo/user `hooks.json` files execute during gwt-managed sessions.
- **FR-023b**: When `GWT_SESSION_RUNTIME_PATH` targets `~/.gwt/sessions/runtime/<gwt-pid>/...`, Codex launch configs also add that PID namespace directory as a writable root so runtime hooks can persist sidecars under `workspace-write` sandboxing.
- **FR-023c**: Embedded runtime-hook distribution must not assume interactive Codex emits `SessionStart` immediately on launch. Downstream launch/bootstrap logic may pre-seed a `Running` sidecar before the first interactive hook event arrives.

### PreToolUse Bash Guard Hooks and Node Runtime-State Hook

- **FR-050**: The PreToolUse `Bash` matcher in both `.claude/settings.local.json` and `.codex/hooks.json` always contains exactly the following gwt-managed entries, in order: `node <scripts>/gwt-block-git-branch-ops.mjs`, `node <scripts>/gwt-block-cd-command.mjs`, `node <scripts>/gwt-block-file-ops.mjs`, `node <scripts>/gwt-block-git-dir-override.mjs`, where `<scripts>` is `.claude/hooks/scripts` or `.codex/hooks/scripts` depending on the target surface.
- **FR-051**: `gwt-block-cd-command.mjs` inspects `tool_input.command` from the PreToolUse payload, parses every `cd` segment separated by `;`, `&&`, `||`, or `|`, resolves the target path relative to the current working directory, and returns a block decision with a reason when at least one resolved target is outside the worktree root returned by `git rev-parse --show-toplevel`.
- **FR-052**: `gwt-block-file-ops.mjs` inspects `tool_input.command` from the PreToolUse payload and returns a block decision when any `mkdir`, `rmdir`, `rm`, `touch`, `cp`, or `mv` invocation resolves at least one operand outside the worktree root. The script must handle option flags (`-r`, `-rf`, `-v`, `--force`, etc.) without treating them as paths.
- **FR-053**: `gwt-block-git-branch-ops.mjs` inspects `tool_input.command` from the PreToolUse payload and returns a block decision for: interactive rebase against `origin/main`; `git checkout <branch>` / `git switch <branch>` that changes the current branch; `git branch -d` / `git branch -D` / `git branch -m` / `git branch <new>`; and `git worktree add|remove|move|prune`. Read-only operations (`git branch --list`, `git branch --show-current`, `git status`, `git log`, `git diff`, `git checkout -- <path>`) must still pass.
- **FR-054**: `gwt-block-git-dir-override.mjs` inspects `tool_input.command` from the PreToolUse payload and returns a block decision when the command contains `GIT_DIR=`, `GIT_WORK_TREE=`, `export GIT_DIR`, `export GIT_WORK_TREE`, `env ... GIT_DIR`, `env ... GIT_WORK_TREE`, `declare -x GIT_DIR`, or `declare -x GIT_WORK_TREE` in any segment.
- **FR-055**: `gwt-runtime-state.mjs` is invoked as `node gwt-runtime-state.mjs <event>` where `<event>` is `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, or `Stop`. It reads `GWT_SESSION_RUNTIME_PATH` from the environment, writes `{status, updated_at, last_activity_at, source_event}` to a temp file and renames it over the sidecar path atomically, and exits 0 whether or not the write succeeded. It must not spawn a secondary process, must not perform network IO, and must not block the agent on error.
- **FR-056**: Gwt's generated `.claude/settings.local.json` and `.codex/hooks.json` emit the same runtime-state command string for POSIX and Windows hosts; the only permitted host-dependent variation is the absolute worktree path prefix. The legacy `posix_runtime_hook_command()` / `powershell_runtime_hook_command()` split is removed.
- **FR-057**: All guard hooks and the runtime-state hook return their decisions through the standard Claude Code hook contract: exit code `2` with a JSON `{ "decision": "block", "reason": "...", "stopReason": "..." }` payload on stdout indicates a block; exit code `0` indicates permit. Guard hooks must not rely on exit codes outside `{0, 2}`.
- **FR-058**: Embedded-asset distribution must always write `gwt-runtime-state.mjs`, `gwt-block-cd-command.mjs`, `gwt-block-file-ops.mjs`, `gwt-block-git-branch-ops.mjs`, and `gwt-block-git-dir-override.mjs` to both `.claude/hooks/scripts/` and `.codex/hooks/scripts/`. `gwt-forward-hook.mjs` must not be written as a bundled active asset; if the tracked repository still carries it, distribution leaves the tracked file alone but does not write a new copy.

## Non-Functional Requirements

- **NFR-001**: Docker detection completes within 2 seconds (check for docker CLI and project files).
- **NFR-002**: Managed Claude/Codex hook regeneration preserves 100% of user-defined hooks in supported regeneration scenarios while never dirtying tracked `.codex/hooks.json` files that are already on the current runtime-hook shape.
- **NFR-003**: Skill distribution to a worktree completes within 1 second.
- **NFR-004**: Binary download via postinstall completes within 60 seconds on a typical connection.
- **NFR-005**: Docker Progress screen updates in real-time (at least 1 update per second during build).

## Implementation Details

### Managed Hook Config Schema

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "node <worktree>/.claude/hooks/scripts/gwt-runtime-state.mjs SessionStart",
            "env": { "GWT_MANAGED_HOOK": "runtime-state" }
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "node <worktree>/.claude/hooks/scripts/gwt-runtime-state.mjs UserPromptSubmit",
            "env": { "GWT_MANAGED_HOOK": "runtime-state" }
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "node <worktree>/.claude/hooks/scripts/gwt-runtime-state.mjs PreToolUse",
            "env": { "GWT_MANAGED_HOOK": "runtime-state" }
          }
        ]
      },
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "node <worktree>/.claude/hooks/scripts/gwt-block-git-branch-ops.mjs" },
          { "type": "command", "command": "node <worktree>/.claude/hooks/scripts/gwt-block-cd-command.mjs" },
          { "type": "command", "command": "node <worktree>/.claude/hooks/scripts/gwt-block-file-ops.mjs" },
          { "type": "command", "command": "node <worktree>/.claude/hooks/scripts/gwt-block-git-dir-override.mjs" }
        ]
      }
    ],
    "PostToolUse": [ /* runtime-state entry only */ ],
    "Stop":        [ /* runtime-state entry only */ ]
  }
}
```

- gwt-managed runtime hooks are identified by the `GWT_MANAGED_HOOK=runtime-state` marker (carried in `env` when the hook schema supports it, and duplicated inside the command string as a fallback for backwards-compatible detection).
- Merge logic: preserve all user hooks, update only gwt-managed runtime-state and guard entries.
- Legacy detector recognizes: `gwt-forward-hook.mjs`, direct `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'`, and `powershell ... GWT_MANAGED_HOOK ... runtime-state ...` commands.
- Codex tracked-file rule: if `.codex/hooks.json` is tracked, generation is skipped unless the file still contains any historical gwt-managed runtime-hook shape.
- Interactive Codex caveat: `SessionStart` may not be emitted before the first prompt, so downstream launch code must tolerate a hook-silent startup window.
- Subprocess guarantee: neither `gwt-runtime-state.mjs` nor the four guard scripts spawn a secondary `gwt`/`gwt-tauri` process.

### Hooks Events

| Event | Description |
|-------|-------------|
| `PreToolUse` | Before agent executes a tool |
| `PostToolUse` | After agent executes a tool |
| `SessionStart` | When the agent session starts |
| `UserPromptSubmit` | When user submits a prompt |
| `Stop` | When agent session stops |

### npm/bunx Distribution

```json
{
  "name": "gwt",
  "bin": { "gwt": "bin/gwt" },
  "scripts": {
    "postinstall": "node scripts/postinstall.js"
  }
}
```

- `postinstall.js`: detects OS/arch, downloads binary from GitHub Release, places in `bin/`
- Supported: macOS arm64/x86_64, Linux x86_64, Windows x86_64

### US-5: Methodology-Based Skill Consolidation (P1) -- NOT IMPLEMENTED

As a developer using gwt, I want the skill system consolidated from 22 skills to 8 methodology-based skills so that I can easily understand which skill to use in each situation.

**Acceptance Scenarios**

1. Given I want to design a feature, when I call `gwt-design`, then it runs DDD-based intake, domain discovery, SPEC registration, and clarification in a single flow.
2. Given I have a clarified spec.md, when I call `gwt-plan`, then it generates SDD architecture, plan.md, tasks.md, and runs the quality gate.
3. Given I want to implement code, when I call `gwt-build` without a SPEC, then it runs TDD Red-Green-Refactor in standalone mode.
4. Given I want to improve code quality, when I call `gwt-review`, then it generates a prioritized architecture improvement report.
5. Given I call any of the 8 new skills, when invoked standalone, then it works without requiring other skills as dependencies.
6. Given the design-plan-build-review chain, when each skill completes, then it suggests the next skill in the feedback loop.

### US-6: DDD Integration in Design Phase (P1) -- NOT IMPLEMENTED

As a developer, I want the design phase to include Domain-Driven Design methodology so that SPEC scope is bounded by domain contexts and avoids cross-boundary complexity.

**Acceptance Scenarios**

1. Given a rough feature request, when `gwt-design` runs Phase 2 (Domain Discovery), then it identifies Bounded Contexts, maps entities, and defines Ubiquitous Language.
2. Given a proposed SPEC scope crosses multiple Bounded Contexts, when the granularity gate runs, then it recommends splitting the SPEC.

### US-7: Architecture Feedback Loop (P1) -- NOT IMPLEMENTED

As a developer, I want a codebase review skill that closes the feedback loop so that code quality improves over time instead of degrading.

**Acceptance Scenarios**

1. Given any repository, when I call `gwt-review`, then it analyzes domain boundaries, module depth, testability, and agent-friendliness.
2. Given the review report, when improvements are identified, then it suggests creating improvement SPECs via `gwt-design`.

### US-8: Search Runtime Contract Recovery (P1) -- IMPLEMENTED

As a developer using `gwt-search`, I want the shared search runtime to repair itself and expose stable action names so that project, issue, and SPEC search keep working across upgrades.

**Acceptance Scenarios**

1. Given `~/.gwt/runtime/chroma_index_runner.py` is missing or outdated, when gwt starts or initializes a workspace, then the repo-tracked runner is restored automatically.
2. Given the managed search venv is missing or broken, when gwt starts or initializes a workspace, then `~/.gwt/runtime/chroma-venv` is rebuilt automatically.
3. Given file search is invoked, when the runner parses CLI args, then `search-files` and `index-files` are the canonical action names.
4. Given legacy callers still use `search` or `index`, when the runner executes, then those aliases are normalized to `search-files` and `index-files`.
5. Given issue indexing is invoked, when the runner executes `index-issues`, then `--project-root` is required in addition to `--db-path`.
6. Given Windows PATH resolves launcher entrypoints first, when gwt chooses a bootstrap Python for the managed search runtime, then it probes them and accepts any candidate that successfully reports Python 3.9+.
7. Given Python candidates exist but are broken or too old, when the managed search runtime cannot be bootstrapped, then gwt surfaces the runtime failure detail instead of misreporting the situation as “Python not installed”.
8. Given the managed search runtime cannot be bootstrapped because no suitable Python candidate exists at all, when gwt surfaces the warning, then the message includes install guidance.
9. Given a user invokes standalone semantic search over project implementation files, when gwt exposes the standalone skill and slash command surface, then `gwt-project-search` is the canonical name.
10. Given the bundled assets are distributed to a worktree, when standalone project search assets are materialized, then no `gwt-file-search` skill or slash-command asset is written.
11. Given `search-files` is used for implementation discovery, when file indexing runs, then embedded skill assets, local SPEC directories, archived SPEC directories, local task logs, and snapshot files are excluded from the implementation-file collection.
12. Given project documentation is indexed separately from implementation files, when `index-files` completes, then `search-files` searches the code-focused collection by default and `search-files-docs` can search the docs-focused collection explicitly.

### US-9: PreToolUse Bash Guard Hooks for Agent Safety (P1) -- IMPLEMENTED

As an operator running an agent inside a worktree, I want every Bash tool call to be screened by gwt-managed PreToolUse hooks so that an agent cannot silently escape the worktree, destroy branches, or repoint Git at an unrelated repository.

**Acceptance Scenarios**

1. Given an agent attempts to run `cd /tmp` (or any path outside the active worktree root), when the Bash PreToolUse hook fires, then `gwt-block-cd-command.mjs` returns a block decision with a reason pointing at the worktree boundary and the agent never executes the command.
2. Given an agent attempts to run `rm -rf ../other` or `mv foo ../outside`, when the Bash PreToolUse hook fires, then `gwt-block-file-ops.mjs` returns a block decision because at least one resolved target is outside the worktree root.
3. Given an agent attempts `git checkout main`, `git switch other`, `git branch -D foo`, `git worktree remove ...`, or an interactive rebase against `origin/main`, when the Bash PreToolUse hook fires, then `gwt-block-git-branch-ops.mjs` returns a block decision while read-only queries like `git branch --list` and `git branch --show-current` still pass.
4. Given an agent attempts to override `GIT_DIR` or `GIT_WORK_TREE` via `GIT_DIR=... git ...`, `export GIT_DIR=...`, `env GIT_DIR=...`, or `declare -x GIT_DIR=...`, when the Bash PreToolUse hook fires, then `gwt-block-git-dir-override.mjs` returns a block decision that prevents Git from operating on an alternate repository.
5. Given the guard hooks are distributed, when `.claude/settings.local.json` and `.codex/hooks.json` are regenerated, then the PreToolUse array for the `Bash` matcher contains `node .../gwt-block-git-branch-ops.mjs`, `node .../gwt-block-cd-command.mjs`, `node .../gwt-block-file-ops.mjs`, and `node .../gwt-block-git-dir-override.mjs` in that order, in addition to the managed runtime-state hook.
6. Given a user already has custom PreToolUse hooks in `.claude/settings.local.json` or `.codex/hooks.json`, when gwt regenerates the managed block, then the user's hooks are preserved and only gwt-managed entries are touched.

### US-10: Unified Node-Based Managed Runtime Hook (P1) -- NOT IMPLEMENTED

As a maintainer of the gwt hook generator, I want every gwt-managed runtime hook (Claude and Codex, POSIX and Windows) to be emitted as a single `node .../gwt-runtime-state.mjs` invocation so that runtime-state logic is authored once in JavaScript instead of being duplicated across `sh -lc` and `powershell` templates, while the no-subprocess-spawn guarantee from earlier regressions is preserved.

**Context:** Earlier regression work banned Node-based forwarders because the legacy `gwt-forward-hook.mjs` spawned a secondary `gwt`/`gwt-tauri` subprocess whose lookup depended on PATH and app bundle layout, which caused interactive Codex runtime sidecars to fail under sandboxing. That failure mode is orthogonal to using `.mjs` to write the sidecar itself. US-10 reintroduces a Node runtime hook that **only** writes `GWT_SESSION_RUNTIME_PATH` atomically and exits, so the forbidden subprocess-spawn behavior stays out of the runtime hook while shell quoting is eliminated.

**Acceptance Scenarios**

1. Given gwt generates a fresh `.claude/settings.local.json` or untracked `.codex/hooks.json`, when the runtime-state command is emitted for `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, and `Stop`, then each command is exactly `node <worktree>/.claude/hooks/scripts/gwt-runtime-state.mjs <event>` (or the `.codex/...` equivalent) with `GWT_MANAGED_HOOK=runtime-state` set via the hook env, and no `sh -lc` or `powershell` fragment appears.
2. Given a worktree was previously generated under the old `sh -lc` runtime hook shape, when the agent is next launched and gwt regenerates `.claude/settings.local.json`, then every historical runtime-state entry is replaced with the Node invocation in a single pass and user hooks are preserved.
3. Given a tracked `.codex/hooks.json` still contains any of (a) `gwt-forward-hook.mjs`, (b) `sh -lc '...GWT_MANAGED_HOOK=runtime-state...'`, or (c) `powershell ... GWT_MANAGED_HOOK ... runtime-state ...`, when the agent is launched, then the legacy detector flags the file and the migration writer replaces only the gwt-managed entries with the unified Node invocation while leaving user hooks untouched.
4. Given `GWT_SESSION_RUNTIME_PATH` is unset at runtime, when `gwt-runtime-state.mjs` runs, then it exits 0 without writing or throwing, so the hook chain never blocks the agent.
5. Given `GWT_SESSION_RUNTIME_PATH` is set, when `gwt-runtime-state.mjs` runs, then it creates the parent directory (best-effort), writes `{status, updated_at, last_activity_at, source_event}` to a temporary file, and atomically renames it over the sidecar path; on any IO error it still exits 0 and the rename is not performed.
6. Given the unified Node runtime hook is generated, when a static check runs over the generated config, then no entry contains `gwt-forward-hook.mjs`, no entry spawns `gwt`, `gwt-tauri`, or any other secondary process, and every managed entry points at `gwt-runtime-state.mjs` or a `gwt-block-*.mjs` script.
7. Given Windows and POSIX share the same generator, when `.claude/settings.local.json` or `.codex/hooks.json` is produced on either platform, then the runtime-state command string is byte-identical (modulo the `<worktree>` path), because there is no platform-conditional shell fallback anymore.

## Functional Requirements (Phase 4: Skill Consolidation)

- **FR-024**: gwt-design runs DDD domain discovery (Bounded Context identification, entity relationships, Ubiquitous Language) in Phase 2.
- **FR-025**: gwt-design uses BC boundary check for SPEC granularity judgment.
- **FR-026**: gwt-plan runs SDD architecture design (component design, interface contracts, sequence descriptions) in Phase 2.
- **FR-027**: gwt-build provides TDD Red-Green-Refactor loop outside the SPEC pipeline (standalone mode).
- **FR-028**: gwt-review generates codebase analysis report (domain boundaries, module depth, testability, agent-friendliness).
- **FR-029**: gwt-review suggests gwt-design for improvement SPECs, closing the feedback loop.
- **FR-030**: gwt-issue auto-detects register/resolve mode from arguments.
- **FR-031**: gwt-pr auto-detects create/check/fix mode from current branch PR state.
- **FR-032**: gwt-search provides unified search across SPECs, Issues, and project files.
- **FR-033**: gwt-agent auto-detects discover/read/send/lifecycle mode from arguments.
- **FR-034**: All 8 skills work standalone without requiring other skills as dependencies.
- **FR-035**: design → plan → build → review automatic chain suggests the next skill on completion.
- **FR-036**: gwt-search runtime assets are repo-tracked and copied into `~/.gwt/runtime/` instead of being edited in place.
- **FR-037**: File search canonical action names are `index-files` and `search-files`; `index` and `search` remain compatibility aliases only.
- **FR-038**: `index-issues` requires both `--project-root` and `--db-path`.
- **FR-039**: Search skill documentation and command examples use the canonical file-search action names and the managed `chroma-venv` path.
- **FR-040**: Search runtime repair uses warning-only degradation when Python or dependency setup fails.
- **FR-041**: Search runtime bootstrap validates Python candidates by executing them and checking for a supported Python 3 runtime before creating the managed venv.
- **FR-042**: Search runtime bootstrap probes launcher candidates by execution and accepts working Python 3.9+ Store/launcher entrypoints instead of rejecting them by path heuristic alone.
- **FR-043**: Search runtime failure guidance tells the user to install Python 3.9+ only when no candidate exists; broken or too-old candidates surface their runtime failure detail.
- **FR-044**: Search runtime bootstrap discovers versioned `python3.x` executables beyond a fixed hard-coded list when they are present on PATH.
- **FR-045**: Startup and clone-completion notifications use the same stable project-index runtime classification rather than brittle human-text matching.
- **FR-046**: `gwt-project-search` is the canonical standalone skill and slash-command name for semantic search over project implementation files, while internal runner actions remain `search-files` / `index-files`.
- **FR-047**: Search-related skill documentation that points users to standalone project-file search references `gwt-project-search` as the primary entrypoint, and `gwt-file-search` is not distributed as a public asset.
- **FR-048**: `index-files` splits indexed project files into separate code and docs collections. `search-files` targets the code-focused collection by default, while `search-files-docs` targets project docs explicitly.
- **FR-049**: The code-focused file collection excludes embedded skill assets (`.claude/`, `.codex/`), local SPEC directories (`specs/`), archived SPEC directories (`specs-archive/`), local task logs (`tasks/`), and snapshot files (`*.snap`) so implementation search is not dominated by generated or planning artifacts.

## Success Criteria

- **SC-001**: GitHub Release produces downloadable binaries for all 4 target platforms.
- **SC-002**: `bunx gwt` successfully downloads and launches gwt on macOS and Linux.
- **SC-003**: Docker Progress screen renders all 5 states with correct transitions.
- **SC-004**: Service Select screen lists services from a test docker-compose.yml.
- **SC-005**: Port Select screen detects and resolves a simulated port conflict.
- **SC-006**: Container start/stop/restart commands execute and report status.
- **SC-007**: After agent launch, all embedded skill files exist in `.claude/skills/` and `.codex/skills/` in the target worktree.
- **SC-011**: build.rs rejects a SKILL.md with malformed YAML frontmatter and produces a clear error message.
- **SC-012**: `.git/info/exclude` contains gwt-managed markers and excludes all distributed asset paths.
- **SC-013**: `.claude/settings.local.json`, untracked `.codex/hooks.json`, and tracked `.codex/hooks.json` files that still contain any historical gwt-managed runtime-hook shape are materialized with the unified Node-based runtime hook and preserve user hooks across consecutive agent launches.
- **SC-014**: All SKILL.md descriptions use third-person voice and include specific trigger phrases.
- **SC-015**: All SKILL.md bodies stay under 500 lines with detailed logic in `references/` subdirectories.
- **SC-008**: Untracked `.codex/hooks.json` regeneration preserves user hooks across consecutive gwt-managed updates.
- **SC-009**: Tracked `.codex/hooks.json` already on the unified Node-based runtime-hook shape remains unchanged after agent launch.
- **SC-010**: Generated Claude/Codex runtime hooks invoke `node .../gwt-runtime-state.mjs` for every managed live-state event, write runtime state through `GWT_SESSION_RUNTIME_PATH`, and contain no `sh -lc`, `powershell`, or `gwt-forward-hook.mjs` gwt-managed entry.
- **SC-022**: A gwt-managed Codex launch includes `--enable codex_hooks`, so Codex runtime hooks execute in both tracked and untracked worktrees.
- **SC-021**: Tracked `.codex/hooks.json` files that still contain any historical gwt-managed runtime-hook shape (legacy forwarder, `sh -lc`, or PowerShell) are migrated to the unified Node-based runtime-hook form before the launched Codex session starts.
- **SC-031**: After `.claude/settings.local.json` is generated for a worktree, running `node .claude/hooks/scripts/gwt-block-cd-command.mjs` with a PreToolUse payload containing `{ "tool_input": { "command": "cd /tmp" } }` exits with code 2 and emits a JSON `decision: "block"` body; the same invocation with `"cd ./src"` exits 0.
- **SC-032**: Running `gwt-block-file-ops.mjs` with `rm -rf ../outside` exits 2, and with `mkdir ./subdir` exits 0.
- **SC-033**: Running `gwt-block-git-branch-ops.mjs` with `git checkout main` exits 2 while `git branch --show-current` and `git status` exit 0.
- **SC-034**: Running `gwt-block-git-dir-override.mjs` with `GIT_DIR=/tmp/repo git status` exits 2 while `git status` alone exits 0.
- **SC-035**: Running `gwt-runtime-state.mjs SessionStart` with `GWT_SESSION_RUNTIME_PATH=/tmp/gwt-sidecar.json` writes a well-formed JSON sidecar atomically, exits 0, and performs no subprocess spawns (verified by process tracing in the integration test).
- **SC-036**: The generator's POSIX and Windows test runs produce byte-identical runtime-state command strings for a fixed worktree path, proving the platform-conditional shell fallback has been removed.
- **SC-037**: `gwt-forward-hook.mjs` is neither referenced by any gwt-managed hook entry in newly generated `.claude/settings.local.json` / `.codex/hooks.json` nor written as a bundled asset; legacy detection still recognizes it for migration purposes.
- **SC-016**: `gwt-design` creates a SPEC with DDD domain model through the full intake-to-clarification flow.
- **SC-017**: `gwt-build` runs TDD Red-Green-Refactor in standalone mode without a SPEC.
- **SC-018**: All 8 skills are callable standalone and produce correct results.
- **SC-019**: `gwt-review` generates an architecture improvement report on the gwt repository.
- **SC-020**: The design → plan → build → review chain suggests the next skill at each completion point.
- **SC-021**: `gwt-search` documentation references `search-files` / `index-files` as the file-search contract.
- **SC-022**: `index-issues` command examples include `--project-root "$GWT_PROJECT_ROOT"`.
- **SC-023**: Deleting the shared runner or managed venv and restarting gwt triggers runtime self-repair instead of leaving search silently broken.
- **SC-024**: On Windows, a PATH entry that resolves to a working Microsoft Store / launcher Python entrypoint is accepted when it reports Python 3.9+.
- **SC-025**: When only broken or too-old Python candidates are present, gwt surfaces runtime failure detail rather than install guidance.
- **SC-026**: When no suitable bootstrap Python is available, gwt surfaces install guidance that references Python 3.9+ and the expected Windows `python` / `py -3` commands.
- **SC-027**: Distributed skill assets include `gwt-project-search` for both Claude and Codex, and `/gwt:gwt-project-search` is available as the canonical slash command.
- **SC-028**: Distributed worktrees do not contain `gwt-file-search` skill or slash-command assets, preventing public naming drift away from the project-search workflow.
- **SC-029**: Reindexing a repository with `.claude/`, `.codex/`, `specs/`, `specs-archive/`, `tasks/`, and snapshot files present leaves those artifacts out of the implementation-file collection while still indexing implementation code.
- **SC-030**: After `index-files`, a query executed through `search-files` returns implementation files without README/spec/skill asset noise, and `search-files-docs` can still retrieve project documentation separately.
