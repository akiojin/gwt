// Agent-window close confirmation guard (user verification 2026-06-12,
// SPEC-2359 W-15 follow-up). Pure logic only — no DOM, no WebSocket.
//
// Mirrors the SPEC-2013 FR-011 agent-pane criteria used by the project tab
// close confirm (`window_is_agent_pane` in runtime_support.rs): a window is
// an agent pane when it carries an `agent_id` or uses an agent preset. The
// close `×` must confirm whenever such a pane still has a live process —
// `running`, `waiting` (alive, awaiting input), or `starting` (spawning) —
// because closing kills the agent. Settled states (`idle`, `stopped`,
// `error`) close silently, matching the project tab behavior of only
// confirming when work would actually be interrupted.

const AGENT_PRESETS = new Set(["agent", "claude", "codex"]);
const LIVE_RUNTIME_STATES = new Set(["running", "waiting", "starting"]);

export function isAgentPaneWindow(windowData) {
  if (!windowData) return false;
  if (windowData.agent_id) return true;
  return AGENT_PRESETS.has(String(windowData.preset || "").toLowerCase());
}

export function shouldConfirmAgentWindowClose(windowData, runtimeState) {
  return (
    isAgentPaneWindow(windowData) &&
    LIVE_RUNTIME_STATES.has(String(runtimeState || "").toLowerCase())
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
