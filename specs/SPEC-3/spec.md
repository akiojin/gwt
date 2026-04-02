# Agent Management -- Detection, Launch Wizard, Custom Agents, Version Cache

## Background

gwt detects and launches coding agents (Claude Code, Codex, Gemini, OpenCode, Copilot) with an 11-step wizard. Agents are detected via PATH lookup combined with `--version` invocation. Custom agents are configurable via the Settings management tab. The launch wizard provides Quick Start from branch history, agent selection, model configuration, and session setup. This SPEC covers the complete agent management domain including detection, wizard flow, custom agents, and the planned version cache feature.

## User Stories

### US-1: Launch Agent via Wizard (P0) -- IMPLEMENTED

As a developer, I want to launch a coding agent through a guided wizard so that I can configure the session correctly before starting.

**Acceptance Scenarios**

1. Given I initiate agent launch, when the wizard starts, then I see the Quick Start step with branch history options.
2. Given I proceed through the wizard, when I reach the Confirm step, then all configured options (agent, model, reasoning level, mode, branch, issue) are summarized.
3. Given I confirm the wizard, when the agent launches, then a new session is created with the configured parameters.
4. Given I cancel at any wizard step, when I press Escape, then no session is created and I return to the previous view.

### US-2: Detect Installed Agents (P0) -- IMPLEMENTED

As a developer, I want gwt to automatically detect which coding agents are installed so that I only see available options.

**Acceptance Scenarios**

1. Given Claude Code is installed and in PATH, when gwt detects agents, then Claude Code appears in the agent selection list.
2. Given an agent is not installed, when gwt detects agents, then that agent does not appear in the selection list.
3. Given an agent binary exists but `--version` fails, when gwt detects agents, then that agent is marked as unavailable with a warning.

### US-3: Quick Start from Branch History (P0) -- IMPLEMENTED

As a developer, I want to quickly re-launch a previous agent session configuration so that I can resume common workflows without re-configuring.

**Acceptance Scenarios**

1. Given I have previously launched an agent on branch `feature/foo`, when I open Quick Start, then the previous configuration for that branch is listed.
2. Given Quick Start history has multiple entries, when I select one, then the wizard pre-fills all fields from the selected history entry.
3. Given Quick Start history is empty, when I open Quick Start, then I see a message indicating no history and can proceed to manual configuration.

### US-4: Manage Custom Agents (P1) -- IMPLEMENTED

As a developer, I want to add, edit, and remove custom agents via Settings so that I can use agents not built into gwt.

**Acceptance Scenarios**

1. Given I am in Settings > Custom Agents, when I add a new agent with display name, type (Command/Path/Bunx), and command, then it appears in the agent selection list.
2. Given a custom agent exists, when I edit its configuration, then the changes are saved and reflected on next use.
3. Given a custom agent exists, when I delete it, then it is removed from the agent selection list.
4. Given a custom agent's command is invalid, when I try to launch it, then an error message is displayed with the failing command.

### US-5: Cache Agent Version List at Startup (P1) -- NOT IMPLEMENTED

As a developer, I want gwt to cache available agent versions at startup so that version selection in the wizard is fast and does not block the UI.

**Acceptance Scenarios**

1. Given gwt starts, when the version cache is empty or expired (TTL 24 hours), then gwt fetches the last 10 versions per agent from the npm registry asynchronously.
2. Given the version cache is fresh, when I open the agent wizard, then version options load instantly from cache.
3. Given the network is unavailable during cache refresh, when I open the wizard, then stale cached versions are shown with a "cache outdated" indicator.
4. Given a new agent version is published, when the cache refreshes after TTL expiry, then the new version appears in the list.

### US-6: Convert Sessions Between Agent Types (P2) -- PARTIALLY IMPLEMENTED

As a developer, I want to convert an existing session to a different agent type so that I can switch tools mid-workflow.

**Acceptance Scenarios**

1. Given an active agent session, when I initiate conversion, then I can select a target agent type from available agents.
2. Given a session conversion is confirmed, when the conversion completes, then the session PTY is replaced with the new agent while preserving the working directory.
3. Given conversion fails (target agent not available), when the error occurs, then the original session remains intact with an error notification.

## Edge Cases

- Agent binary exists in PATH but is a broken symlink.
- Multiple versions of the same agent installed (e.g., via nvm, different PATH entries).
- Custom agent command contains spaces or special characters in the path.
- npm registry returns unexpected JSON format during version cache fetch.
- Version cache file is corrupted on disk.
- Network timeout during version fetch (should not block startup).
- Quick Start history file grows very large (hundreds of entries).
- Agent detection runs concurrently with user opening the wizard.
- Session conversion attempted while the session has active PTY I/O.

## Functional Requirements

- **FR-001**: `AgentTrait::detect()` checks PATH for agent binary and invokes `--version` to confirm availability.
- **FR-002**: `AgentLaunchBuilder` constructs launch configuration including model, fast_mode, reasoning_level, and environment variables.
- **FR-003**: Wizard flow proceeds through steps: QuickStart, Agent, Model, Reasoning, Mode, Branch, Issue, Confirm.
- **FR-004**: Custom agent CRUD operations available in Settings > Custom Agents tab.
- **FR-005**: `CustomCodingAgent` structure: id, display_name, agent_type (Command/Path/Bunx), command string.
- **FR-006**: Version list cache fetches last 10 versions per agent from npm registry on startup.
- **FR-007**: Cache stored in `~/.gwt/cache/agent-versions.json` with 24-hour TTL.
- **FR-008**: Quick Start stores per-branch launch history in persistent storage.
- **FR-009**: Session resume via `agent_session_id` for agents that support session continuity.
- **FR-010**: Codex hooks confirmation flow integrated into the wizard when Codex agent is selected.
- **FR-011**: Agent detection timeout: individual agent detection must complete within 5 seconds.
- **FR-012**: Version cache fetch is async and non-blocking; does not delay startup or wizard display.

## Non-Functional Requirements

- **NFR-001**: Total agent detection for all known agents completes under 2 seconds.
- **NFR-002**: Version cache fetch runs asynchronously and does not block UI rendering.
- **NFR-003**: Cache file I/O uses atomic write (write to temp, rename) to prevent corruption.
- **NFR-004**: Quick Start history is bounded to 100 entries per branch to limit file size.
- **NFR-005**: Custom agent configuration changes are persisted immediately (no explicit save step).

## Success Criteria

- **SC-001**: All known agents (Claude Code, Codex, Gemini, OpenCode, Copilot) are correctly detected when installed.
- **SC-002**: Launch wizard completes without errors for all agent types.
- **SC-003**: Custom agent CRUD works end-to-end via Settings UI.
- **SC-004**: Version cache fetches, stores, and serves cached versions correctly.
- **SC-005**: Version cache gracefully degrades when network is unavailable.
- **SC-006**: Quick Start history correctly records and retrieves per-branch configurations.
- **SC-007**: Session conversion preserves working directory and handles errors gracefully.
