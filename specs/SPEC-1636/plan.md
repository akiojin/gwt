# Plan

## Summary

Implement cancel-aware Assistant runs on the backend, wire transcript reply actions and composer delivery modes on the frontend, and update the canonical Assistant spec artifacts.

## Technical Context

- Backend orchestration: `crates/gwt-tauri/src/commands/assistant.rs`
- Engine/tool loop: `crates/gwt-tauri/src/assistant_engine.rs`
- Runtime state: `crates/gwt-tauri/src/state.rs`
- Frontend composer/transcript: `gwt-gui/src/lib/components/AssistantPanel.svelte`

## Implementation Approach

1. Add window-scoped Assistant runtime state for active run generation and queued inputs
2. Convert user sends from synchronous request/response execution to spawn-and-emit execution
3. Make engine runs cancel-aware so stale runs stop before committing assistant/tool output
4. Route Enter/Send/reply-button through interrupt mode and Tab through queue mode
5. Surface queue count in Assistant state and render it in the composer area
6. Verify with targeted Rust tests, AssistantPanel tests, and `svelte-check`
