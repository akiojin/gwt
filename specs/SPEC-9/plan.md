# SPEC-9: Infrastructure -- Implementation Plan

## Phase 1: Docker UI Restoration

**Goal**: Restore Docker integration screens from the old TUI (v6.30.3) to the current ratatui-based TUI.

### Approach

Reference the old TUI implementation files (`docker_progress.rs`, `service_select.rs`, `port_select.rs`) for design patterns and state machine structure. Reimplement using current ratatui widget patterns and the existing screen navigation system.

### Key Changes

1. **gwt-tui**: Add `DockerProgress` screen with 5-state FSM (DetectingFiles, BuildingImage, StartingContainer, WaitingForServices, Ready).
   - Each state renders a progress indicator and status message.
   - Transitions driven by async events from gwt-core `DockerManager`.

2. **gwt-tui**: Add `ServiceSelect` screen.
   - Parse docker-compose.yml via gwt-core to list services.
   - Selectable list with service name and image.

3. **gwt-tui**: Add `PortSelect` screen.
   - Display conflicting ports with current and proposed mappings.
   - Allow user to edit port mappings inline.

4. **gwt-core**: Ensure `DockerManager` exposes async event stream for progress states.
   - File detection, image build, container start, service readiness checks.

5. **gwt-tui**: Add container lifecycle controls (start/stop/restart) accessible from the Docker status area.

### Dependencies

- Existing `DockerManager` in gwt-core.
- docker CLI available on the system.

## Phase 2: Embedded Skills

**Goal**: Implement skill registration on startup, add a pre-SPEC brainstorming entrypoint, and expose embedded skill state in the UI.

### Key Changes

1. **gwt-core**: Define `EmbeddedSkill` struct with name, description, entry point, and status.
   - Registry: `Vec<EmbeddedSkill>` populated at startup.
   - Skills: gwt-pr, gwt-pr-check, gwt-pr-fix, gwt-spec-brainstorm, gwt-spec-ops, etc.
   - Bundled gwt-spec skills stay aligned with the local SPEC artifact model, including persisted `analysis.md`.

2. **embedded skills**: Add `gwt-spec-brainstorm` as the cross-agent pre-SPEC intake skill and expose `/gwt:gwt-spec-brainstorm` as the Claude command entrypoint.
   - One-question-at-a-time interview flow.
   - Duplicate search across Issues and SPECs before new registration.
   - Auto-handoff to `gwt-spec-ops`, `gwt-spec-register`, or `gwt-issue-register`.

3. **gwt-tui**: Add skill management panel (accessible from Settings or a dedicated screen).
   - List registered skills with status indicators.
   - Allow enable/disable per skill.

4. **gwt-core**: Extend `gwt-pr-check` to produce structured status report.
   - CI check status (pass/fail/pending per check).
   - Merge readiness (conflicts, approvals, required checks).
   - Review thread states (resolved/unresolved counts).

### Dependencies

- Existing skill definitions in `.claude/skills/`.
- GitHub API access for gwt-pr-check.

## Phase 3: Hooks Merge Completion

**Goal**: Complete the hooks.json merge feature carried over from archived SPEC-1786 (20/31 tasks completed, remaining: Phase 3 Polish and Phase 4 Manual E2E).

### Carried-Over Progress

The following capabilities from SPEC-1786 are already implemented:

- `write_managed_codex_hooks()` with merge mode.
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

- Existing hooks merge implementation in gwt-core.

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
