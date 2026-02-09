# Codex collaboration_modes Support

**SPEC ID**: `SPEC-fdebd681`
**Created**: 2026-01-26
**Status**: Approved
**カテゴリ**: Porting

**Implementation Phase**: Phase 2 (Implementation)

## User Scenarios and Tests *(Required)*

### User Story 1 - Auto-enable collaboration_modes for Codex v0.91.0+ (Priority: P1)

When a developer selects Codex v0.91.0 or later, collaboration_modes is
automatically enabled without requiring user selection (Plan mode support).

**Reason for priority**: Enables gwt users to leverage a new Codex feature
with zero friction.

**Independent test**: Select Codex v0.91.0+ -> CollaborationModes step skipped
-> `--enable collaboration_modes` automatically included in CLI args.

**Acceptance scenarios**:

1. **Given** Codex v0.91.0+ is selected, **When** VersionSelect completes,
   **Then** CollaborationModes step is skipped and collaboration_modes is
   automatically set to true.
2. **Given** Codex v0.91.0+ is selected,
   **When** wizard completes, **Then** Codex launch args include
   `--enable collaboration_modes` automatically.

---

### User Story 2 - Skip CollaborationModes step for Codex v0.90.x (Priority: P1)

When a developer selects Codex v0.90.x or earlier, the CollaborationModes step
is skipped because the feature is not supported.

**Reason for priority**: Prevents errors on unsupported versions.

**Independent test**: Select Codex v0.90.0 -> CollaborationModes step not shown
-> Transition directly to ExecutionMode.

**Acceptance scenarios**:

1. **Given** Codex v0.90.0 selected, **When** VersionSelect completes,
   **Then** CollaborationModes step skipped, transition to ExecutionMode.
2. **Given** Codex v0.89.x selected, **When** VersionSelect completes,
   **Then** CollaborationModes step skipped.

---

### User Story 3 - Show CollaborationModes for "latest" version (Priority: P1)

When a developer selects the "latest" version, the TUI assumes the latest
version supports collaboration_modes and shows the step.

**Reason for priority**: Latest version is assumed to be v0.91.0+ capable.

**Independent test**: Select version "latest" -> CollaborationModes step shown.

**Acceptance scenarios**:

1. **Given** version "latest" selected, **When** VersionSelect completes,
   **Then** CollaborationModes step is displayed.

---

### User Story 4 - Skip CollaborationModes for Claude Code/Gemini (Priority: P2)

When a developer selects Claude Code or Gemini, the CollaborationModes step
is skipped because it is a Codex-specific feature.

**Reason for priority**: Codex-specific feature should not appear for others.

**Independent test**: Select Claude Code -> CollaborationModes step not shown.

**Acceptance scenarios**:

1. **Given** Claude Code selected, **When** VersionSelect completes,
   **Then** CollaborationModes step skipped.
2. **Given** Gemini selected, **When** VersionSelect completes,
   **Then** CollaborationModes step skipped.

---

### Edge Cases

- "installed" selection: If installed version is unknown, do not show
  CollaborationModes (safe default).
- Quick Start restores collaboration_modes setting.
- Session history entries without collaboration_modes field treated as false.

## Requirements *(Required)*

### Functional Requirements

- **FR-001**: gwt shall automatically enable collaboration_modes for Codex
  v0.91.0+ (step is skipped, no user selection required).
- **FR-002**: gwt shall automatically enable collaboration_modes for version
  "latest" (assumed to be v0.91.0+ capable).
- **FR-003**: gwt shall check installed version for "installed" selection
  and auto-enable collaboration_modes only if v0.91.0+.
- **FR-004**: gwt shall add `--enable collaboration_modes` to Codex args
  automatically when v0.91.0+ is selected.
- **FR-005**: gwt shall skip CollaborationModes step for non-Codex agents.
- **FR-006**: gwt shall persist collaboration_modes in ToolSessionEntry.

### Main Entities

- **AgentLaunchConfig**: Add `collaboration_modes: bool` field.
- **WizardState**: Add `collaboration_modes: bool` field.
- **ToolSessionEntry**: Add `collaboration_modes: Option<bool>` field.

## Success Criteria *(Required)*

### Measurable Outcomes

- **SC-001**: collaboration_modes auto-enabled 100% for Codex v0.91.0+.
- **SC-002**: CollaborationModes step skipped 100% for all versions (auto-enable).
- **SC-003**: `--enable collaboration_modes` added 100% for Codex v0.91.0+.
- **SC-004**: Regression tests pass for Claude Code/Gemini workflows.

## Out of Scope *(Required)*

- Plan/Execute mode switching UI inside collaboration_modes (Codex feature).
- Detailed collaboration_modes settings (model/reasoning_effort override,
  see Codex Issue #9783).
- collaboration_modes support for non-Codex agents.

## Dependencies *(If applicable)*

- Codex CLI v0.91.0+ with `--enable collaboration_modes` flag.
- Existing version parsing logic (`parse_version`, `compare_versions`).
- Existing WizardStep transition logic.

## References *(If applicable)*

- [OpenAI Codex Releases](https://github.com/openai/codex/releases)
- [GitHub Issue #9783](https://github.com/openai/codex/issues/9783)
- [GitHub Discussion #7355](https://github.com/openai/codex/discussions/7355)
