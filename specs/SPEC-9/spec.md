# Infrastructure -- Build Distribution, Docker UI, Embedded Skills, Hooks Merge

## Background

gwt infrastructure covers four domains: build/distribution (GitHub Release + bunx/npx), Docker integration UI (detection, container lifecycle, port mapping), embedded skill management, and Codex hooks.json merge. Docker UI screens existed in the old TUI (v6.30.3) and need restoration to the current ratatui-based TUI. The hooks.json merge feature was 65% complete (20/31 tasks done) in the archived SPEC-1786 before it was consolidated into this SPEC. Embedded skill management also owns keeping the bundled `.claude/skills/gwt-*` assets aligned with the current local SPEC artifact model, including persisted `analysis.md`, and now covers the pre-SPEC intake entrypoint that interviews rough requests before any `spec.md` is drafted.

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

### US-3: Distribute Embedded Skills to Worktrees on Agent Launch (P1) -- NOT IMPLEMENTED

As a developer, I want gwt to bundle all embedded skills, commands, and hooks into the binary and distribute them to the target worktree every time an agent is launched, so that agents always have up-to-date skill definitions without manual configuration.

**Acceptance Scenarios**

1. Given an agent is launched from gwt, when the launch completes, then `.claude/skills/`, `.claude/commands/`, `.claude/hooks/`, `.codex/skills/` are written to the target worktree with the bundled skill files.
2. Given the target worktree already has older untracked gwt-managed skill files, when an agent is launched, then those generated files are overwritten with the latest bundled versions.
3. Given the target worktree tracks `.claude/*` or `.codex/*` gwt asset paths in Git, when an agent is launched, then distribution preserves those tracked files and only writes untracked generated targets.
4. Given an agent is launched, when skill distribution completes, then `.claude/settings.local.json` is generated with gwt-managed hooks, preserving any existing user-defined hooks via merge logic.
5. Given an agent is launched, when skill distribution completes, then `.git/info/exclude` in the worktree is updated to exclude gwt-managed asset paths (`.claude/skills/gwt-*`, `.claude/commands/gwt-*`, `.claude/hooks/scripts/gwt-*`, `.codex/skills/gwt-*`, `.claude/settings.local.json`).
6. Given the gwt binary is built, when build.rs runs, then all SKILL.md files are validated for YAML frontmatter syntax errors, and the build fails with a clear error if any SKILL.md has malformed YAML.
7. Given all skills are bundled, when the binary starts, then no runtime file I/O is needed to read skill definitions — skills are embedded in the binary via `include_dir`.

### US-4: Merge hooks.json Preserving User Hooks (P1) -- PARTIALLY IMPLEMENTED

As a developer, I want gwt to merge its managed hooks into hooks.json without overwriting my custom hooks so that both gwt automation and my personal hooks coexist.

**Acceptance Scenarios**

1. Given hooks.json contains user-defined hooks, when gwt updates its managed hooks, then user hooks are preserved.
2. Given gwt-managed hooks are identified by a comment marker, when merging, then only gwt-managed entries are updated.
3. Given hooks.json is corrupted, when gwt attempts to merge, then a backup is created and recovery is attempted.
4. Given a Codex agent session is starting, when hooks need to be written, then a confirmation dialog is shown.

## Edge Cases

- Docker daemon not running when Docker workflow is selected.
- docker-compose.yml references images that do not exist locally.
- Port conflict on a privileged port (below 1024).
- hooks.json contains syntax errors or is not valid JSON.
- hooks.json is a symlink to a shared configuration.
- Multiple gwt instances attempting concurrent hooks.json merge.
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

- **FR-009**: All skill, command, and hook files under `.claude/skills/`, `.claude/commands/`, `.claude/hooks/scripts/` are embedded into the gwt binary at build time using `include_dir` crate. Embedded skill categories:
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
- **FR-013**: Distribution overwrites untracked gwt-managed generated files on each agent launch.
- **FR-013a**: Distribution must skip writes for gwt-managed asset paths that are already tracked by Git in the target worktree.
- **FR-014**: `.claude/settings.local.json` is generated on each agent launch. gwt-managed hooks are merged using `hooks.rs` merge logic, preserving user-defined hooks.
- **FR-015**: `.git/info/exclude` is updated on each agent launch to exclude gwt-managed asset paths. Existing user entries are preserved; gwt-managed entries are delimited by `# gwt-managed-begin` / `# gwt-managed-end` markers.

### Embedded Skills — Quality Standards (Anthropic Guidelines)

- **FR-016**: All SKILL.md `description` fields follow Anthropic guidelines: third-person voice, specific trigger phrases, front-loaded key use case within 250 characters.
- **FR-017**: All SKILL.md body content uses imperative/infinitive form, stays under 500 lines, and delegates detailed logic to `references/` subdirectories (Progressive Disclosure).
- **FR-018**: All SKILL.md frontmatter actively uses `allowed-tools`, `argument-hint`, and other applicable fields as defined by the Claude Code skill specification.

### Hooks Merge (carried over from archived SPEC-1786)

- **FR-019**: `write_managed_codex_hooks()` uses merge mode: read existing hooks.json, update only gwt-managed entries, write back.
- **FR-020**: Preserve user-defined hooks during gwt-managed hook updates; never delete or modify entries without the gwt marker.
- **FR-021**: gwt-managed hooks identified by a `"_gwt_managed": true` field on each managed hook entry.
- **FR-022**: Confirmation dialog displayed for Codex agent sessions only before writing hooks.
- **FR-023**: JSON corruption recovery: on parse failure, create timestamped backup, attempt recovery from last known good state, and fall back to writing gwt-only hooks if recovery fails.

## Non-Functional Requirements

- **NFR-001**: Docker detection completes within 2 seconds (check for docker CLI and project files).
- **NFR-002**: Hooks merge preserves 100% of user-defined hooks in all scenarios including corruption recovery.
- **NFR-003**: Skill distribution to a worktree completes within 1 second.
- **NFR-004**: Binary download via postinstall completes within 60 seconds on a typical connection.
- **NFR-005**: Docker Progress screen updates in real-time (at least 1 update per second during build).

## Implementation Details

### hooks.json Schema

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "/path/to/gwt-hook pre-tool $TOOL_NAME",
            "// gwt-managed": true
          }
        ]
      }
    ],
    "PostToolUse": [...],
    "UserPromptSubmit": [...],
    "Notification": [...],
    "Stop": [...]
  }
}
```

- gwt-managed hooks identified by `"// gwt-managed": true` comment field
- Merge logic: preserve all user hooks (without gwt-managed marker), update gwt-managed hooks
- On corruption: backup to `hooks.json.bak`, write fresh managed hooks

### Hooks Events

| Event | Description |
|-------|-------------|
| `PreToolUse` | Before agent executes a tool |
| `PostToolUse` | After agent executes a tool |
| `UserPromptSubmit` | When user submits a prompt |
| `Notification` | On notification event |
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
- **SC-013**: `.claude/settings.local.json` is generated with gwt-managed hooks and preserves user hooks across consecutive agent launches.
- **SC-014**: All SKILL.md descriptions use third-person voice and include specific trigger phrases.
- **SC-015**: All SKILL.md bodies stay under 500 lines with detailed logic in `references/` subdirectories.
- **SC-008**: hooks.json merge preserves user hooks across 10 consecutive gwt-managed updates.
- **SC-009**: hooks.json corruption recovery creates backup and restores functionality.
- **SC-010**: All carried-over hooks merge tests from SPEC-1786 continue to pass.
- **SC-016**: `gwt-design` creates a SPEC with DDD domain model through the full intake-to-clarification flow.
- **SC-017**: `gwt-build` runs TDD Red-Green-Refactor in standalone mode without a SPEC.
- **SC-018**: All 8 skills are callable standalone and produce correct results.
- **SC-019**: `gwt-review` generates an architecture improvement report on the gwt repository.
- **SC-020**: The design → plan → build → review chain suggests the next skill at each completion point.
- **SC-021**: `gwt-search` documentation references `search-files` / `index-files` as the file-search contract.
- **SC-022**: `index-issues` command examples include `--project-root "$GWT_PROJECT_ROOT"`.
- **SC-023**: Deleting the shared runner or managed venv and restarting gwt triggers runtime self-repair instead of leaving search silently broken.
