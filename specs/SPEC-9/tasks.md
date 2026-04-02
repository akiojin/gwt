# SPEC-9: Infrastructure -- Tasks

## Phase 1: Docker UI Restoration

### 1.1 Docker Progress Screen

- [ ] **T-001**: Write test for DockerProgress screen 5-state FSM transitions (DetectingFiles -> BuildingImage -> StartingContainer -> WaitingForServices -> Ready).
- [ ] **T-002**: Write test for DockerProgress screen rendering at each state with correct status message.
- [ ] **T-003**: Write test for DockerProgress error state handling (Docker daemon not running).
- [ ] **T-004**: Implement `DockerProgress` screen in `crates/gwt-tui/src/screens/docker_progress.rs`.
- [ ] **T-005**: Wire DockerProgress screen to DockerManager async event stream.
- [ ] **T-006**: Verify T-001 through T-003 pass (GREEN).

### 1.2 Service Select Screen

- [ ] **T-007**: Write test for ServiceSelect screen listing services from docker-compose.yml.
- [ ] **T-008**: Write test for ServiceSelect with single service auto-selecting.
- [ ] **T-009**: Write test for ServiceSelect with no services showing error message.
- [ ] **T-010**: Implement `ServiceSelect` screen in `crates/gwt-tui/src/screens/service_select.rs`.
- [ ] **T-011**: Verify T-007 through T-009 pass (GREEN).

### 1.3 Port Select Screen

- [ ] **T-012**: Write test for PortSelect screen detecting port conflicts.
- [ ] **T-013**: Write test for PortSelect allowing port remap.
- [ ] **T-014**: Write test for PortSelect with no conflicts proceeding automatically.
- [ ] **T-015**: Implement `PortSelect` screen in `crates/gwt-tui/src/screens/port_select.rs`.
- [ ] **T-016**: Verify T-012 through T-014 pass (GREEN).

### 1.4 Container Lifecycle Controls

- [ ] **T-017**: Write test for container start command execution.
- [ ] **T-018**: Write test for container stop command execution.
- [ ] **T-019**: Write test for container restart command execution.
- [ ] **T-020**: Implement container lifecycle controls in Docker status area.
- [ ] **T-021**: Verify T-017 through T-019 pass (GREEN).

## Phase 2: Embedded Skills

### 2.1 Skill Registration

- [P] [ ] **T-022**: Write test for EmbeddedSkill struct fields (name, description, entry_point, status).
- [P] [ ] **T-023**: Write test for skill registry populated with expected skills on startup.
- [P] [ ] **T-024**: Write test for partial registration failure (one skill fails, others succeed).
- [ ] **T-025**: Implement `EmbeddedSkill` struct and registry in `crates/gwt-core/src/skills.rs`.
- [ ] **T-026**: Register gwt-pr, gwt-pr-check, gwt-pr-fix, gwt-spec-ops at startup.
- [ ] **T-027**: Verify T-022 through T-024 pass (GREEN).

### 2.2 Skill Management UI

- [ ] **T-028**: Write test for skill management panel rendering with registered skills.
- [ ] **T-029**: Write test for skill enable/disable toggle.
- [ ] **T-030**: Implement skill management panel in gwt-tui.
- [ ] **T-031**: Verify T-028, T-029 pass (GREEN).

### 2.3 gwt-pr-check Extended Report

- [ ] **T-032**: Write test for structured status report containing CI, merge, and review states.
- [ ] **T-033**: Write test for report when PR has no checks (empty CI section).
- [ ] **T-034**: Implement extended gwt-pr-check status report in gwt-core.
- [ ] **T-035**: Verify T-032, T-033 pass (GREEN).

## Phase 3: Hooks Merge Completion (carried over from SPEC-1786)

> Progress: 20/31 tasks from SPEC-1786 completed. Tasks below are the remaining 11.

### 3.1 Core Merge Logic (COMPLETED from SPEC-1786)

- [x] **T-036**: write_managed_codex_hooks() reads existing hooks.json before writing.
- [x] **T-037**: Managed hooks identified by `_gwt_managed: true` marker.
- [x] **T-038**: User-defined hooks preserved during merge.
- [x] **T-039**: New managed hooks appended without duplicating existing ones.
- [x] **T-040**: Removed managed hooks cleaned up from hooks.json.
- [x] **T-041**: Merge handles empty hooks.json (fresh file).
- [x] **T-042**: Merge handles missing hooks.json (creates new file).

### 3.2 Confirmation Dialog (COMPLETED from SPEC-1786)

- [x] **T-043**: Confirmation dialog shown for Codex agent sessions.
- [x] **T-044**: Non-Codex agent sessions skip confirmation.
- [x] **T-045**: User can cancel hook writing from confirmation dialog.

### 3.3 Basic Error Handling (COMPLETED from SPEC-1786)

- [x] **T-046**: Invalid JSON in hooks.json detected before merge.
- [x] **T-047**: Error message displayed to user on parse failure.
- [x] **T-048**: Write failure rolls back to previous state.

### 3.4 Advanced Hooks Array Handling (COMPLETED from SPEC-1786)

- [x] **T-049**: Hooks with same event type merged correctly.
- [x] **T-050**: Hook ordering preserved (user hooks first, managed hooks after).
- [x] **T-051**: Duplicate managed hook detection and dedup.
- [x] **T-052**: Hook entry validation (required fields present).
- [x] **T-053**: Large hooks.json (100+ entries) handled without performance degradation.
- [x] **T-054**: Unicode content in hook commands preserved.
- [x] **T-055**: Nested JSON structures in hook configs preserved.

### 3.5 Polish (remaining from SPEC-1786 Phase 3)

- [x] **T-056**: Write test for timestamped backup creation on corruption detection.
- [x] **T-057**: Write test for last-known-good restoration after backup.
- [x] **T-058**: Write test for concurrent write handling (file lock contention).
- [x] **T-059**: Write test for symlinked hooks.json merge behavior.
- [x] **T-060**: Write test for empty hooks.json file (0 bytes) recovery.
- [x] **T-061**: Implement timestamped backup and recovery logic.
- [x] **T-062**: Implement file locking for concurrent write prevention.
- [x] **T-063**: Improve error messages for merge failure scenarios.
- [x] **T-064**: Verify T-056 through T-060 pass (GREEN).

### 3.6 Manual E2E Verification (remaining from SPEC-1786 Phase 4)

- [ ] **T-065**: Manual E2E: merge across 10 consecutive gwt-managed updates, verify all user hooks preserved.
- [ ] **T-066**: Manual E2E: inject JSON corruption, verify backup created and recovery succeeds.

## Phase 4: Build Distribution

### 4.1 GitHub Release Workflow

- [ ] **T-067**: Write test for release workflow matrix producing 4 platform binaries.
- [ ] **T-068**: Write test for Conventional Commits version detection (feat=minor, fix=patch, !=major).
- [ ] **T-069**: Verify release workflow configuration in `.github/workflows/release.yml`.
- [ ] **T-070**: Verify git-cliff CHANGELOG generation from commit history.
- [ ] **T-071**: Verify T-067, T-068 pass (GREEN).

### 4.2 npm Distribution

- [ ] **T-072**: Write test for postinstall script detecting platform and architecture.
- [ ] **T-073**: Write test for postinstall script downloading correct binary.
- [ ] **T-074**: Write test for postinstall script handling download failure gracefully.
- [ ] **T-075**: Verify postinstall script on macOS arm64.
- [ ] **T-076**: Verify postinstall script on Linux x86_64.
- [ ] **T-077**: Verify T-072 through T-074 pass (GREEN).
