# Feature Specification: Migrate from Tauri GUI to ratatui TUI

## Background

gwt is a SPEC-driven agent management tool that launches coding agents (Claude Code, Codex, Gemini) against SPECs with git worktrees providing isolated workspaces. The current frontend is a Tauri v2 + Svelte 5 desktop GUI introduced in v7.0.0.

This SPEC replaces the GUI with a ratatui-based TUI, making gwt a terminal application that serves as a terminal replacement with its own tab management. Users launch gwt instead of a terminal app and manage all agents from within it. The core value proposition (SPEC-driven agent management with automatic worktree isolation) remains unchanged.

Design document: \`docs/superpowers/specs/2026-03-27-tui-migration-design.md\`

## User Stories

### User Story 1 - Launch gwt as terminal replacement (Priority: P1)

As a developer, I want to launch gwt from my shell and have it replace my terminal application, so that I can manage all coding agents and shell sessions from a single TUI interface.

### User Story 2 - Manage agent tabs (Priority: P1)

As a developer, I want to create, switch between, and close agent tabs within gwt, so that I can run multiple coding agents simultaneously without opening multiple terminal windows.

### User Story 3 - Use shell tabs (Priority: P1)

As a developer, I want to open plain shell tabs alongside agent tabs, so that I can use gwt as my complete terminal solution.

### User Story 4 - View management panel (Priority: P1)

As a developer, I want to toggle a management panel with Ctrl+G to see all agents' status, SPEC associations, and quick actions, so that I can monitor and control agents without leaving the TUI.

### User Story 5 - Split panes (Priority: P2)

As a developer, I want to split the terminal view horizontally or vertically, so that I can monitor multiple agents side by side like tmux.

### User Story 6 - View PR and Issue status (Priority: P2)

As a developer, I want to see PR status, CI results, and Issue/SPEC information in the management panel, so that I can track progress without switching to a browser.

### User Story 7 - AI session summaries (Priority: P2)

As a developer, I want to see AI-generated summaries of each agent's scrollback, so that I can quickly understand what each agent is doing without reading full terminal output.

### User Story 8 - Use voice input (Priority: P3)

As a developer, I want to use voice input (Qwen3-ASR) to send commands to the active terminal tab, so that I can interact hands-free.

## Acceptance Scenarios

1. Given gwt is launched from a shell, when it starts, then a ratatui TUI is displayed with a tab bar, terminal area, and status bar.
2. Given the TUI is running, when the user presses Ctrl+G then n, then a new agent launch dialog appears allowing agent type and branch selection.
3. Given an agent tab is active, when the user types, then keystrokes are forwarded to the agent's PTY.
4. Given multiple tabs exist, when the user presses Ctrl+G then 1-9, then the corresponding tab becomes active.
5. Given the TUI is running, when the user presses Ctrl+G, then the management panel toggles showing agent list, detail, and quick actions.
6. Given the management panel is visible, when the user selects an agent and presses Enter, then the TUI switches to that agent's tab.
7. Given split mode is active, when two agents run side by side, then both PTY outputs render correctly in their respective panes.
8. Given an agent is launched with an Issue/branch, then a worktree is automatically created and the agent runs within it.
9. Given an agent tab is closed, then the associated worktree is cleaned up (with safety checks).
10. Given the TUI is running, when the terminal is resized, then all panes and the tab bar resize correctly.

## Edge Cases

- Terminal size below 80x24: display warning, disable split mode
- Agent PTY crash: tab remains with error indicator, restart option available
- Worktree creation failure (disk full, permissions): error shown in management panel, tab not created
- GitHub API unreachable: PR/Issue panels show offline state, background retry
- Rapid tab switching: debounce rendering to prevent flicker
- Ctrl+G conflicts: prefix key should not be forwarded to PTY under any circumstances

## Functional Requirements

- FR-001: gwt-tui crate using ratatui + crossterm replaces gwt-tauri + gwt-gui
- FR-002: Tab bar with agent name, branch, and status color indicators
- FR-003: Full PTY terminal rendering with ANSI color and attribute support
- FR-004: Ctrl+G prefix key system for all management operations
- FR-005: Management panel with agent list, detail view, and quick actions (kill, restart, logs)
- FR-006: New agent launch dialog with agent type, branch/Issue, and directory selection
- FR-007: Shell tab support (opens default shell in current or specified directory)
- FR-008: Horizontal and vertical pane splitting
- FR-009: Status bar showing current tab info, SPEC association, and agent state
- FR-010: Scrollback buffer with scroll mode (Ctrl+G, PgUp) and file persistence
- FR-011: PR dashboard in management panel (status, CI checks, merge state)
- FR-012: Issue/SPEC list in management panel with search
- FR-013: AI session summary display in management panel
- FR-014: Voice input integration (Qwen3-ASR)
- FR-015: Automatic worktree creation on agent launch and cleanup on close
- FR-016: VT100 emulator buffer to ratatui Cell conversion (renderer)

## Non-Functional Requirements

- NFR-001: Rendering latency under 16ms per frame (60fps capable)
- NFR-002: Memory usage proportional to scrollback buffer size, not unbounded
- NFR-003: Cross-platform support (macOS, Linux, Windows) via crossterm
- NFR-004: Startup time under 500ms to first interactive frame
- NFR-005: gwt-core changes limited to business logic migration from gwt-tauri; no breaking API changes

## Success Criteria

- SC-001: gwt launches as a TUI application and displays a functional tab bar + terminal
- SC-002: Users can create, switch, and close both agent and shell tabs
- SC-003: Ctrl+G management panel shows accurate agent status and allows control operations
- SC-004: Split panes render correctly with independent PTY sessions
- SC-005: All existing gwt-core tests pass without modification
- SC-006: gwt-tui has >80% test coverage on renderer, keybind, and state modules
- SC-007: gwt-tauri and gwt-gui are fully removed from the repository
- SC-008: CI/release pipeline updated for TUI binary distribution
