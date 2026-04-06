# Infrastructure -- Build Distribution, Docker UI, Embedded Skills, Hooks Merge

## Background

gwt infrastructure covers four domains: build/distribution (GitHub Release + bunx/npx), Docker integration UI (detection, container lifecycle, port mapping), embedded skill management, and managed hook configuration for Claude Code / Codex. Docker UI screens existed in the old TUI (v6.30.3) and need restoration to the current ratatui-based TUI. The older archived hooks.json merge work from SPEC-1786 remains as a generic utility in `hooks.rs`, but the active Claude/Codex runtime-hook path is now a typed config generator that writes `.claude/settings.local.json` and `.codex/hooks.json`, preserves user hooks, preserves tracked Codex hook files by default, migrates tracked files that still contain legacy gwt-managed runtime forwarders, and emits no-Node live-state commands that write `GWT_SESSION_RUNTIME_PATH`. Embedded skill management also owns keeping the bundled `.claude/skills/gwt-*` assets aligned with the current local SPEC artifact model, including persisted `analysis.md`, and now covers the pre-SPEC intake entrypoint that interviews rough requests before any `spec.md` is drafted.

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
4. Given an agent is launched, when skill distribution completes, then `.claude/settings.local.json` and `.codex/hooks.json` for untracked worktrees or tracked worktrees that still carry legacy gwt-managed runtime forward hooks are generated or migrated with gwt-managed runtime hooks, preserving any existing user-defined hooks while replacing only gwt-managed runtime entries.
5. Given an agent is launched, when skill distribution completes, then `.git/info/exclude` in the worktree is updated to exclude gwt-managed asset paths (`.claude/skills/gwt-*`, `.claude/commands/gwt-*`, `.claude/hooks/scripts/gwt-*`, `.codex/skills/gwt-*`, `.codex/hooks/scripts/gwt-*`, `.claude/settings.local.json`, `.codex/hooks.json`).
6. Given the gwt binary is built, when build.rs runs, then all SKILL.md files are validated for YAML frontmatter syntax errors, and the build fails with a clear error if any SKILL.md has malformed YAML.
7. Given all skills are bundled, when the binary starts, then no runtime file I/O is needed to read skill definitions — skills are embedded in the binary via `include_dir`.

### US-4: Generate Managed Claude/Codex Hook Configs Preserving User Hooks (P1) -- IMPLEMENTED

As a developer, I want gwt to generate managed Claude/Codex hook configs without overwriting my custom hooks so that both gwt automation and my personal hooks coexist.

**Acceptance Scenarios**

1. Given `.claude/settings.local.json` or an untracked `.codex/hooks.json` contains user-defined hooks, when gwt updates its managed runtime hooks, then user hooks are preserved.
2. Given a prior config contains stale gwt-managed runtime hooks, when gwt regenerates the file, then only the gwt-managed runtime entries are replaced.
3. Given Codex runtime hooks are generated for an untracked worktree, when the file is written, then live-state hook commands update `GWT_SESSION_RUNTIME_PATH` directly without a Node-based forwarder.
4. Given `.codex/hooks.json` is tracked by Git in the target worktree and contains no legacy gwt-managed runtime forward hooks, when an agent launches, then gwt does not rewrite that file and does not dirty tracked source files.
5. Given `.codex/hooks.json` is tracked by Git in the target worktree and still contains legacy gwt-managed runtime forward hooks, when an agent launches, then gwt migrates only the gwt-managed runtime entries to the current no-Node form while preserving user hooks.
6. Given gwt launches a Codex agent session, when the launch command is built, then Codex starts with the `codex_hooks` feature enabled so `hooks.json` actually executes.

## Edge Cases

- Docker daemon not running when Docker workflow is selected.
- docker-compose.yml references images that do not exist locally.
- Port conflict on a privileged port (below 1024).
- `.claude/settings.local.json` or `.codex/hooks.json` contains invalid JSON and must be treated as a recoverable empty-object input.
- `.codex/hooks.json` is tracked by Git in the target worktree and contains no legacy gwt-managed runtime forward hooks; it must not be rewritten.
- `.codex/hooks.json` is tracked by Git in the target worktree and still contains legacy gwt-managed runtime forward hooks; those gwt-managed runtime entries must be migrated without dropping user hooks.
- Codex has `hooks.json` available but the `codex_hooks` feature flag is not enabled at launch.
- Multiple gwt instances are running simultaneously; runtime hook commands must use the injected `GWT_SESSION_RUNTIME_PATH` instead of recomputing shared global paths.
- Target worktree is read-only or has insufficient disk space for skill distribution.
- `.git/info/exclude` does not exist (must be created).
- `.claude/settings.local.json` contains user-defined hooks that conflict with gwt-managed hooks.
- SKILL.md frontmatter contains YAML syntax errors (caught at build time).
- Agent launch is interrupted mid-distribution (partial write).
- Target worktree tracks bundled `.claude/*` or `.codex/*` assets in Git; distribution must not dirty tracked source files.
- npm postinstall script runs in an environment without internet access.
- GitHub Release workflow runs but binary compilation fails on one platform.

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
  - Agent pane management: gwt-agent-discover, gwt-agent-read, gwt-agent-send, gwt-agent-lifecycle
  - Utilities: gwt-project-search, gwt-spec-to-issue-migration
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
- **FR-013a**: Distribution must skip writes for gwt-managed asset paths that are already tracked by Git in the target worktree.
- **FR-014**: `.claude/settings.local.json` is generated on each agent launch from a typed hook-config builder that preserves non-gwt hooks and unrelated Claude settings while replacing only gwt-managed runtime hooks.
- **FR-014a**: `.codex/hooks.json` is generated on each agent launch when the file is untracked in the target worktree. Existing user hooks are preserved, gwt-managed runtime hooks are replaced, and tracked `.codex/hooks.json` files are left untouched unless they still contain legacy gwt-managed runtime forward hooks.
- **FR-015**: `.git/info/exclude` is updated on each agent launch to exclude gwt-managed asset paths, including `.codex/hooks.json`. Existing user entries are preserved; gwt-managed entries are delimited by `# gwt-managed-begin` / `# gwt-managed-end` markers.

### Embedded Skills — Quality Standards (Anthropic Guidelines)

- **FR-016**: All SKILL.md `description` fields follow Anthropic guidelines: third-person voice, specific trigger phrases, front-loaded key use case within 250 characters.
- **FR-017**: All SKILL.md body content uses imperative/infinitive form, stays under 500 lines, and delegates detailed logic to `references/` subdirectories (Progressive Disclosure).
- **FR-018**: All SKILL.md frontmatter actively uses `allowed-tools`, `argument-hint`, and other applicable fields as defined by the Claude Code skill specification.

### Managed Runtime Hook Generation

- **FR-019**: Claude and Codex runtime hook configs are generated from a shared typed builder so both surfaces emit the same live-state event mapping and hook ordering.
- **FR-020**: Preserve user-defined hooks during gwt-managed runtime hook updates; only gwt-managed runtime entries are replaced.
- **FR-021**: gwt-managed runtime hooks are identified by a command marker (`GWT_MANAGED_HOOK`) and legacy forward-hook command patterns during config sanitization.
- **FR-022**: Live-state runtime hooks write directly to `GWT_SESSION_RUNTIME_PATH` and do not spawn Node-based runtime forwarders or `gwt hook` subprocesses.
- **FR-023**: If `.codex/hooks.json` is tracked by Git in the target worktree, gwt preserves the tracked file unchanged unless it still contains legacy gwt-managed runtime forward hooks; in that case, gwt migrates only the gwt-managed runtime entries to the current no-Node form while preserving user hooks.
- **FR-024**: Codex launch configs generated by gwt enable the `codex_hooks` feature flag so repo/user `hooks.json` files execute during gwt-managed sessions.
- **FR-024a**: When `GWT_SESSION_RUNTIME_PATH` targets `~/.gwt/sessions/runtime/<gwt-pid>/...`, Codex launch configs also add that PID namespace directory as a writable root so runtime hooks can persist sidecars under `workspace-write` sandboxing.

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
            "command": "GWT_MANAGED_HOOK=runtime-state sh -lc '...write $GWT_SESSION_RUNTIME_PATH...'"
          }
        ]
      }
    ],
    "UserPromptSubmit": [...],
    "PreToolUse": [...],
    "PostToolUse": [...],
    "Stop": [...]
  }
}
```

- gwt-managed runtime hooks are identified by the `GWT_MANAGED_HOOK` command marker
- Merge logic: preserve all user hooks, update only gwt-managed runtime hooks
- Codex tracked-file rule: if `.codex/hooks.json` is tracked, generation is skipped unless the file still contains gwt's legacy runtime forward-hook commands

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
- **SC-013**: `.claude/settings.local.json`, untracked `.codex/hooks.json`, and tracked `.codex/hooks.json` files that still contain legacy gwt runtime forward hooks are materialized with gwt-managed runtime hooks and preserve user hooks across consecutive agent launches.
- **SC-014**: All SKILL.md descriptions use third-person voice and include specific trigger phrases.
- **SC-015**: All SKILL.md bodies stay under 500 lines with detailed logic in `references/` subdirectories.
- **SC-008**: Untracked `.codex/hooks.json` regeneration preserves user hooks across consecutive gwt-managed updates.
- **SC-009**: Tracked `.codex/hooks.json` without legacy gwt runtime forward hooks remains unchanged after agent launch.
- **SC-010**: Generated Claude/Codex runtime hooks contain no Node-based live-state forward command and write runtime state through `GWT_SESSION_RUNTIME_PATH`.
- **SC-014**: A gwt-managed Codex launch includes `--enable codex_hooks`, so Codex runtime hooks execute in both tracked and untracked worktrees.
- **SC-021**: Tracked `.codex/hooks.json` files that still contain legacy gwt runtime forward hooks are migrated to the no-Node runtime-hook shape before the launched Codex session starts.
- **SC-016**: `gwt-design` creates a SPEC with DDD domain model through the full intake-to-clarification flow.
- **SC-017**: `gwt-build` runs TDD Red-Green-Refactor in standalone mode without a SPEC.
- **SC-018**: All 8 skills are callable standalone and produce correct results.
- **SC-019**: `gwt-review` generates an architecture improvement report on the gwt repository.
- **SC-020**: The design → plan → build → review chain suggests the next skill at each completion point.
