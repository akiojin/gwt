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

### US-3: Manage Embedded Skills Registration and Pre-SPEC Intake (P1) -- NOT IMPLEMENTED

As a developer, I want gwt to register its embedded skills on startup and expose a pre-SPEC brainstorming entrypoint so that AI agents can handle rough requests without manual configuration or premature SPEC creation.

**Acceptance Scenarios**

1. Given gwt starts, when initialization completes, then all embedded skills (gwt-pr, gwt-pr-check, gwt-pr-fix, gwt-spec-brainstorm, etc.) are registered.
2. Given I open the skill management panel, when I view registered skills, then each skill shows its name, description, and status.
3. Given gwt-pr-check is invoked, when it runs, then it reports CI status, merge readiness, and review state in a structured format.
4. Given the local SPEC workflow changes its persisted artifact model, when embedded gwt-spec skills are refreshed, then the bundled skill docs stay aligned with that model (including `analysis.md`).
5. Given a user starts with a rough idea or asks whether an existing SPEC should be updated, when `gwt-spec-brainstorm` is invoked, then it performs one-question-at-a-time intake, searches existing owners first, and routes existing-Issue matches into `gwt-issue-resolve` and existing-SPEC updates into `gwt-spec-ops` instead of creating duplicate artifacts prematurely.

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
- Embedded skill registration fails for one skill (partial registration).
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

### Embedded Skills

- **FR-009**: Skill registration on startup: register gwt-pr, gwt-pr-check, gwt-pr-fix, `gwt-spec-brainstorm`, and other embedded skills, and keep embedded gwt-spec workflow docs aligned with the local SPEC artifact model.
- **FR-010**: `gwt-spec-brainstorm` must provide a cross-agent pre-SPEC intake workflow that performs duplicate search first, interviews the user one question at a time, and routes to `EXISTING-ISSUE` via `gwt-issue-resolve`, `EXISTING-SPEC` via `gwt-spec-ops`, `NEW-SPEC`, or `ISSUE` before any new `spec.md` is drafted.
- **FR-011**: Skill management UI: display registered skills with name, description, and status in a settings panel or dedicated screen.
- **FR-012**: gwt-pr-check extended status report: CI check status, merge readiness, review thread states, combined in a structured output.

### Hooks Merge (carried over from archived SPEC-1786)

- **FR-013**: `write_managed_codex_hooks()` uses merge mode: read existing hooks.json, update only gwt-managed entries, write back.
- **FR-014**: Preserve user-defined hooks during gwt-managed hook updates; never delete or modify entries without the gwt marker.
- **FR-015**: gwt-managed hooks identified by a `"_gwt_managed": true` field on each managed hook entry.
- **FR-016**: Confirmation dialog displayed for Codex agent sessions only before writing hooks.
- **FR-017**: JSON corruption recovery: on parse failure, create timestamped backup, attempt recovery from last known good state, and fall back to writing gwt-only hooks if recovery fails.

## Non-Functional Requirements

- **NFR-001**: Docker detection completes within 2 seconds (check for docker CLI and project files).
- **NFR-002**: Hooks merge preserves 100% of user-defined hooks in all scenarios including corruption recovery.
- **NFR-003**: Skill registration completes within 1 second at startup.
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

## Success Criteria

- **SC-001**: GitHub Release produces downloadable binaries for all 4 target platforms.
- **SC-002**: `bunx gwt` successfully downloads and launches gwt on macOS and Linux.
- **SC-003**: Docker Progress screen renders all 5 states with correct transitions.
- **SC-004**: Service Select screen lists services from a test docker-compose.yml.
- **SC-005**: Port Select screen detects and resolves a simulated port conflict.
- **SC-006**: Container start/stop/restart commands execute and report status.
- **SC-007**: All embedded skills, including `gwt-spec-brainstorm`, are registered and queryable after startup.
- **SC-008**: hooks.json merge preserves user hooks across 10 consecutive gwt-managed updates.
- **SC-009**: hooks.json corruption recovery creates backup and restores functionality.
- **SC-010**: All carried-over hooks merge tests from SPEC-1786 continue to pass.
