# Feature Specification: Electron Full Scratch Migration

## Background

Tauri v2 + WKWebView の IPC アーキテクチャにより、メインスレッドがブロックされ
UI が操作不能になる問題が発生している。`list_terminals` 等の高頻度 IPC 呼び出しが
WKWebView の URL scheme handler を経由するため、HTTP IPC へのルーティング等の
ワークアラウンドでも根本解決に至らない。

本 SPEC では Tauri を Electron に全面置換し、Rust バックエンドをサイドカープロセス
(axum HTTP/WebSocket サーバー) として維持する構成に移行する。
フロントエンドは Svelte 5 で新規作成する。

### Architecture

```text
Electron Main Process
  ├── Rust sidecar lifecycle (child_process.spawn)
  ├── Native menu / tray / dialogs
  ├── Window management (BrowserWindow)
  └── Auto-update (electron-updater)

Renderer (Svelte 5 + Vite)
  ├── HTTP POST → gwt-server (commands)
  └── WebSocket ← gwt-server (events)

gwt-server (Rust, standalone binary)
  ├── axum HTTP API (all 158+ commands)
  ├── WebSocket event broadcasting
  ├── gwt-core (git, PTY, AI, config)
  └── AppState (Arc<AppState>, no Tauri dependency)
```

## User Stories

### User Story 1 - Desktop App Startup (Priority: P1)

As a developer, I want the Electron app to start, automatically launch the Rust
sidecar server, and display the main UI within 3 seconds, so that I can begin
working without delay.

**Acceptance Scenario:**
1. Given the app is installed
2. When I launch it
3. Then the main window appears within 3 seconds
4. And the Rust sidecar is running and accepting HTTP requests
5. And the WebSocket connection is established

### User Story 2 - Terminal Operations (Priority: P1)

As a developer, I want to spawn agent/terminal sessions and see real-time PTY
output in xterm.js tiles, so that I can interact with CLI agents.

**Acceptance Scenario:**
1. Given the app is running with a project open
2. When I launch an agent session
3. Then a terminal tile appears in the Agent Canvas
4. And PTY output streams in real-time via WebSocket
5. And I can type input that is sent to the PTY

### User Story 3 - Agent Canvas Interaction (Priority: P1)

As a developer, I want to drag, pan, and zoom tiles on the Agent Canvas
(Figma-style), so that I can organize my workspace visually.

**Acceptance Scenario:**
1. Given the Agent Canvas is displayed with tiles
2. When I drag a tile's handle
3. Then the tile moves smoothly following the pointer
4. When I drag the canvas background
5. Then the viewport pans smoothly
6. When I Ctrl+scroll
7. Then the canvas zooms in/out

### User Story 4 - Branch Browser (Priority: P2)

As a developer, I want to browse branches, view branch details, and manage
worktrees from the GUI, so that I can manage my git workflow visually.

### User Story 5 - Settings and Configuration (Priority: P2)

As a developer, I want to configure app settings (AI models, themes, agent
configs), so that I can customize the tool for my workflow.

### User Story 6 - No IPC Loop Bug (Priority: P1)

As a developer, I want the app to never freeze due to runaway IPC calls,
so that the UI remains responsive at all times.

**Acceptance Scenario:**
1. Given the app is running
2. When I open a project with multiple worktrees and agents
3. Then CPU usage remains below 10% at idle
4. And no IPC command is called more than once per user action or server event

## Edge Cases

- Rust sidecar crashes → Electron detects exit, shows error, offers restart
- Sidecar port conflict → retry with random port
- WebSocket disconnect → auto-reconnect with exponential backoff
- Large terminal output burst → WebSocket backpressure handling
- Multiple Electron windows → single sidecar shared across windows

## Functional Requirements

- FR-001: Electron app launches Rust sidecar via `child_process.spawn`
- FR-002: All Rust commands accessible via HTTP POST to sidecar
- FR-003: All server events delivered via WebSocket
- FR-004: Terminal output streams via WebSocket binary frames
- FR-005: Native menu matches current Tauri menu structure
- FR-006: System tray with Show/Quit actions
- FR-007: File open dialog via Electron dialog API
- FR-008: External URL opening via Electron shell API
- FR-009: Window title management via Electron BrowserWindow
- FR-010: Auto-update via electron-updater
- FR-011: macOS code signing and notarization
- FR-012: Windows MSI packaging
- FR-013: Linux AppImage packaging
- FR-014: Single-instance enforcement
- FR-015: Frontend has zero Tauri dependency
- FR-016: No `$effect` may directly invoke IPC — all IPC is action/event driven
- FR-017: API client layer includes per-command throttling

## Non-Functional Requirements

- NFR-001: App startup to interactive UI < 3 seconds
- NFR-002: Terminal output latency (PTY → screen) < 16ms (60fps)
- NFR-003: Idle CPU usage < 5%
- NFR-004: No IPC command called > 10 times/second without explicit user action
- NFR-005: gwt-core crate remains unchanged (zero modifications)
- NFR-006: Sidecar binary size < 50MB
- NFR-007: Electron app total package < 200MB

## Success Criteria

- SC-001: `cargo tauri dev` replaced by `pnpm electron:dev` — app launches and is interactive
- SC-002: All existing E2E scenarios pass on Electron (adapted from Playwright tests)
- SC-003: Terminal output renders in real-time without UI freeze
- SC-004: Agent Canvas D&D / pan / zoom operates smoothly
- SC-005: CPU at idle < 5% (validated via `ps aux` monitoring)
- SC-006: macOS .dmg, Windows .msi, Linux .AppImage build successfully in CI
- SC-007: No `@tauri-apps` import exists anywhere in the final codebase
