# Research: SPEC-1776 — Electron Migration

## Key Decision: Sidecar vs napi-rs vs Full Rewrite

| Criterion | Sidecar (HTTP/WS) | napi-rs | TypeScript Rewrite |
|---|---|---|---|
| Migration effort | 3-5 weeks | 8-12 weeks | 16-24 weeks |
| Risk | Low | High | Very High |
| gwt-core changes | None | Binding layer needed | Full rewrite |
| PTY throughput | Excellent (separate process) | Good (in-process) | Good (node-pty) |
| Maintenance | Low-Med | High (FFI boundary) | Low (single lang) |

**Decision**: Sidecar approach chosen. Rationale:
1. gwt-core remains unchanged (zero risk)
2. HTTP IPC pattern already proven with 10 endpoints
3. Separate process eliminates ALL main thread blocking
4. WebSocket for events is well-understood

## WKWebView Main Thread Problem (Root Cause)

- Tauri v2 uses WKWebView on macOS
- WKWebView's URL scheme handler processes IPC responses on the main thread
- Even with `spawn_blocking`, the response serialization and delivery blocks the main thread
- High-frequency commands like `list_terminals` (600+ calls/sec from `$effect` loop) make UI completely unresponsive
- HTTP IPC routing partially bypasses this but doesn't cover all commands
- **Conclusion**: The problem is architectural and cannot be fixed within Tauri v2

## Electron IPC Architecture

- Electron uses Chromium (multi-process: main + renderer)
- IPC between main and renderer is asynchronous and non-blocking
- With sidecar approach, no IPC goes through Electron at all for commands
- Frontend directly HTTP-fetches the Rust server (localhost TCP)
- Events arrive via WebSocket (also direct TCP)
- **Result**: Zero main thread contention

## WebSocket Design for Terminal Output

- Terminal output is the highest-frequency event (~60fps for active terminals)
- Binary frames (opcode 0x2) for PTY byte streams
- Text frames (opcode 0x1) for structured JSON events
- Single WS connection per renderer, multiplexed by event type
- Message format: `{ "event": "terminal-output", "pane_id": "...", "data": base64 }`
- For binary: `[1-byte event type][pane_id length][pane_id bytes][payload bytes]`

## Frontend `$effect` Loop Prevention

The current Tauri app has a bug where `list_terminals` is called 600+ times/second:
- Svelte 5 `$effect` tracks implicit dependencies
- A function called within an effect that reads/writes reactive state causes re-triggering
- **Design rule for new frontend**: No IPC call inside `$effect`. All IPC is triggered by:
  1. User actions (click, input, etc.)
  2. WebSocket events (server push)
  3. Explicit timers with guards (debounced polling)

## electron-vite vs Manual Vite Setup

- electron-vite provides out-of-box Svelte 5 + Electron integration
- Handles main/preload/renderer build targets automatically
- HMR for renderer process
- **Decision**: Use electron-vite for project scaffolding
