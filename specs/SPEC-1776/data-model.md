# Data Model: SPEC-1776 — Electron Migration

## AppState (gwt-server)

Migrated from `crates/gwt-tauri/src/state.rs`. Tauri types removed.

```rust
pub struct AppState {
    // PTY & Terminal
    pub pane_manager: Mutex<PaneManager>,
    pub pane_launch_meta: Mutex<HashMap<String, PaneLaunchMeta>>,
    pub pane_runtime_contexts: Mutex<HashMap<String, PaneRuntimeContext>>,
    pub launch_jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub launch_results: Mutex<HashMap<String, LaunchFinishedPayload>>,

    // Caching
    pub agent_versions_cache: Mutex<HashMap<String, AgentVersionsCache>>,
    pub session_summary_cache: Mutex<HashMap<String, SessionSummaryCache>>,
    pub project_version_history_cache: Mutex<HashMap<String, VersionHistoryCacheEntry>>,
    pub project_issue_list_cache: Mutex<HashMap<String, IssueListCacheEntry>>,
    pub project_branch_inventory_snapshot_cache: Mutex<HashMap<String, BranchInventorySnapshotCacheEntry>>,

    // Async tracking
    pub session_summary_inflight: Mutex<HashSet<String>>,
    pub project_version_history_inflight: Mutex<HashSet<String>>,
    pub version_history_semaphore: Arc<Semaphore>,

    // System
    pub system_monitor: Mutex<Option<SystemMonitor>>,
    pub os_env: RwLock<HashMap<String, String>>,
    pub os_env_ready: AtomicBool,

    // Event broadcasting (replaces app_handle.emit())
    pub event_broadcaster: EventBroadcaster,

    // Window tracking (simplified for Electron)
    pub window_projects: Mutex<HashMap<String, String>>,

    // Application state
    pub is_quitting: AtomicBool,
    pub gh_available: AtomicBool,
    pub http_port: AtomicU16,
}
```

## EventBroadcaster

Replaces `app_handle.emit()` with WebSocket-compatible broadcasting.

```rust
pub struct EventBroadcaster {
    sender: broadcast::Sender<ServerEvent>,
}

pub struct ServerEvent {
    pub event: String,           // "terminal-output", "launch-progress", etc.
    pub target: EventTarget,     // All, Window(label), Pane(id)
    pub payload: serde_json::Value,
}

pub enum EventTarget {
    All,
    Window(String),
    Pane(String),
}
```

## HTTP API Request/Response

### Command Request (HTTP POST)

```typescript
// POST http://localhost:{port}/{command_name}
// Content-Type: application/json
{
  "projectPath": "/path/to/project",
  "branchName": "main",
  // ... command-specific fields
}
```

### Command Response

```typescript
// Success: 200 OK
{ /* command-specific response */ }

// Error: 500 Internal Server Error
{
  "severity": "error",
  "code": "E9002",
  "message": "...",
  "command": "list_terminals",
  "category": "Internal",
  "suggestions": [],
  "timestamp": "2026-03-27T12:00:00Z"
}
```

### WebSocket Event Frame

```typescript
// Text frame (structured events)
{
  "event": "launch-progress",
  "target": { "type": "all" },
  "payload": { "jobId": "...", "step": "fetch", "progress": 0.5 }
}

// Binary frame (terminal output)
// [event_type: u8][pane_id_len: u8][pane_id: bytes][data: bytes]
```

## Frontend State (Svelte stores)

```typescript
// lib/stores/project.ts
export const projectPath = writable<string | null>(null);
export const currentBranch = writable<string>("");
export const worktrees = writable<WorktreeInfo[]>([]);

// lib/stores/terminals.ts
export const terminals = writable<TerminalInfo[]>([]);
export const activeTerminalId = writable<string | null>(null);

// lib/stores/canvas.ts
export const canvasViewport = writable<Viewport>({ x: 0, y: 0, zoom: 1 });
export const tileLayouts = writable<Record<string, TileLayout>>({});
export const selectedTileId = writable<string>("assistant");

// lib/stores/tabs.ts
export const tabs = writable<Tab[]>([]);
export const activeTabId = writable<string | null>(null);
```

## Electron Preload API

```typescript
// Exposed via contextBridge
interface ElectronAPI {
  sidecarPort: number;
  appVersion: string;
  platform: NodeJS.Platform;
  dialog: {
    openFile(options: OpenDialogOptions): Promise<string | null>;
    showMessage(options: MessageBoxOptions): Promise<number>;
  };
  shell: {
    openExternal(url: string): Promise<void>;
  };
  window: {
    setTitle(title: string): void;
    minimize(): void;
    maximize(): void;
    close(): void;
  };
  onMenuAction(callback: (action: string) => void): () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
```
