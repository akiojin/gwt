// Agent-window close confirmation guard (user verification 2026-06-12,
// SPEC-2359 W-15 follow-up). Pure logic only — no DOM, no WebSocket.
//
// Mirrors the SPEC-2013 FR-011 agent-pane criteria used by the project tab
// close confirm (`window_is_agent_pane` in runtime_support.rs): a window is
// an agent pane when it carries an `agent_id` or uses an agent preset. The
// close `×` must confirm whenever such a pane still has a live process —
// in the runtime state vocabulary everything except `stopped` / `error`
// (`idle` and `waiting` agents are alive and hold conversation context;
// closing kills them). Only definitively dead panes close silently.

const AGENT_PRESETS = new Set(["agent", "claude", "codex"]);
const DEAD_RUNTIME_STATES = new Set(["stopped", "error"]);

export function isAgentPaneWindow(windowData) {
  if (!windowData) return false;
  if (windowData.agent_id) return true;
  return AGENT_PRESETS.has(String(windowData.preset || "").toLowerCase());
}

export function shouldConfirmAgentWindowClose(windowData, runtimeState) {
  return (
    isAgentPaneWindow(windowData) &&
    !DEAD_RUNTIME_STATES.has(String(runtimeState || "running").toLowerCase())
  );
}

// Display name for the confirm modal, same precedence as the backend's
// RunningAgentSummary (`dynamic_title` → `purpose_title` → `title`).
export function agentWindowDisplayName(windowData) {
  if (!windowData) return "agent";
  return (
    windowData.dynamic_title ||
    windowData.purpose_title ||
    windowData.title ||
    "agent"
  );
}
